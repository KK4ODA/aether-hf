# ADR-0105: The receive filter is read at the signal's edges

**Status:** accepted, 2026-11-09: the author's decision at the receive-filter review's Gate 4,
with the answers to `docs/research/receive-filter/04-results.md` §11. To be built at Gate 5:
the daemon, the panel and the manual. **No wire change, no configuration key, no schema bump.**
The control API's `metrics` changes (§2.6).

## 1. Context

The panel has warned since beta.14 when the radio's receive filter is narrower than the modem's
signal. The warning comes from an estimate of the passband read from the noise between signals
(`spectrum::PassbandMonitor`). It has been fixed twice. d1b0992 took its looks only from whole
windows of quiet, so a tone frame's start was no longer read as a 650 Hz filter. beta.131 took
each bin's 10th percentile over 120 looks, so a band of FT8 was no longer read as an 850 Hz filter.

On the night of 2026-11-07 the mobile's IF SHIFT was turned fully left. The 2300 Hz signal's top
600 Hz arrived 16–30 dB down, and the panel said nothing. That was the third fault, and by the
author's rule a third fault gets a review, not a patch (`docs/research/receive-filter/`).
The review found the estimate saw the cut and judged it wrongly:

* it drew the edge halfway to the profile's floor, 7–25 dB down the skirt;
* it judged the passband's width, so a passband moved by an IF shift, or a low edge set high,
  never warned;
* it told the operator to widen the filter whatever was wrong;
* it needed whole windows of quiet, which a band of FT8 gives a minute or two apart.

On the corpus it named 8 of 48 cuts of 20 dB or more (5 of 48 on a fresh draw), and on the
harness none of 24.

What a cut costs was measured on the harness (`04-results.md` §6.1, the bottom cuts at their real
depths):
* a cut of 10 dB at an edge of the signal costs nothing, at either edge of either bandwidth;
* a cut of 20 dB costs at the 2300 Hz bottom (24–30 % of the Test's message), at the 500 Hz top
  (10 % of the message and 16 % of the file over ten draws), and a step on the 2300 Hz top's
  file;
* at the 500 Hz bottom a 20 dB cut costs nothing; 42 dB costs most of the session.

ADR-0104 made the receivers read the noise across the band, so a cut now costs less than it did,
but it still costs, and the operator can turn the knob back. Every recording of Saturday also
clipped (2.6–3.8 % of the samples heard), and the panel's clipping line was in a card Compact
hides.

## 2. Decision

### 2.1 The reading

Every half second that the station listens (not keyed, not deaf after keying, not counting itself
as receiving a frame), the station takes a spectrum of its last 85 ms of audio up to 10 kHz. Then:

1. **The bins a signal holds are left out**: a bin 14 dB or more over the lower quartile of the
   150 Hz on each side of it, and the two bins beside it (the Hann window's main lobe).
2. **The look is held** for the air's longest frame of any family and half a second, and dropped
   if a frame announced or reported by then lay under it. A frame's spectrum ends at the signal's
   own edges, and would read as a cut.
3. **The look is divided by its own level**, the ninth decile of the bins left in the signal's
   span, and kept only if that level stands 12 dB or more over the audio chain's floor: the median
   of 6–10 kHz, where no radio passes audio. A look whose band noise has sunk to the chain's floor
   (an AGC's dip, low audio) says nothing about the filter.
4. **The profile** is each bin's median over the last 120 looks kept, taken from the looks the
   bin was free in, and only where it was free in half of them. It is smoothed by a running median
   over ±50 Hz, wherever half that window is present.
5. **The reading** is the drop at each edge of the signal's span below the smoothed profile's
   ninth decile over the span. The span is the waveform's own: its centre ± half its occupied
   width (360–2640 Hz, 1260–1740 Hz). A drop of **12 dB** or more names that side: top, bottom,
   or both. Twelve kept looks are needed before there is a reading. If twelve looks are left out
   for the floor before twelve are kept, the reading is *can't check*.

The constants are eleven, against today's twelve:
* the line (12 dB);
* the edge smoothing (±50 Hz);
* the floor's band (6–10 kHz);
* the looks needed (12) and kept (120);
* for the leave-out: the threshold (14 dB), the sides' quantile (the lower quartile), their width
  (150 Hz) and the main lobe (2 bins);
* the level's quantile (the ninth decile);
* the look interval (0.5 s, `PASSBAND_SAMPLE_S`, kept).

The hold is the air's longest frame, read from the air. The line sits between 10 dB, which costs
nothing, and 20 dB, which costs. Over the mobile's shallow chain floor (15 dB under the band's
noise) a 20 dB cut reads about 14 dB and a 10 dB cut about 9, so a line at 12 dB separates both
floors.

### 2.2 Clipping (C1)

C1 is the share of samples at full scale (0.99 or more, as the Receive level tile counts them)
over the last 10 s of the audio the station heard. **Over 0.1 % the audio clips**, and that is a
warning of its own. The passband reading goes on while the audio clips: the veto registered in
`03-candidates.md` §2.3 is dropped (the author's answer 1). With the veto, no clipped and cut
session could ever warn of its cut, and the reading read clipped audio right everywhere it was
tried:
* all twelve cut hot clips of the corpus named, on the right side;
* none of the forty uncut hot clips warned;
* Saturday's top named at 27.7–29.3 dB while 2.6–3.8 % of the samples clipped.

### 2.3 Where it shows

* **A banner in every view, Compact included**, as the session and mismatch banners are, for a cut
  and for clipping. It names what to turn, in plain words:
  * a top cut: *The radio's receive filter cuts the top of the signal, about N dB down: the IF
    shift, the high cut, or the filter width.*
  * a bottom cut: *… the bottom of the signal …: the IF shift, the low cut, or the filter width.*
  * both: *… both edges of the signal …: the filter width, or run the modem at 500 Hz.*
    The last part is only at 2300 Hz.
  * clipping: *The receive audio is clipping: turn the radio's audio output or the sound card's
    input down.*
  Each banner can be dismissed, and comes back when its reading changes: another side, 3 dB more,
  or another bandwidth.
* **A line in the Status tab's readings** for the receive filter: what it says now (passes the
  signal, top/bottom/both cut with the drop, waiting, or *can't check, band noise too weak*).
  *Can't check* is said there quietly and never as a banner (the author's answer at Gate 2).
* The warning is the panel's only. A host program is told nothing of it.

### 2.4 The frames' witness goes

The other station's decoded frames measure its transmit filter and the path as well as this
radio's filter, so they could only bring forward a warning the noise agrees with (the author's
answer at Gate 2). They brought none forward. On the field the first frame spanning the cut
edge came minutes after the noise had named the cut. On the harness it came first in 4 runs of 21,
each with no reading yet to agree with. W1 is not built (the author's answer 3).
`tools/passband_witness.py` stays as an instrument for reading a recording.

### 2.5 The 2026-11-07 15:06Z recording

It was listed as a working station. The reading puts its top 14–16 dB down, and both witnesses of
that afternoon agree, so it counts in the free band between 10 and 20 dB, where the criteria ask
nothing (the author's answer 2).

### 2.6 The control API and the recordings

* `metrics.rx_passband_hz` and `metrics.occupied_hz` are replaced by:
  * `metrics.rx_filter`: `verdict` (`waiting`, `unknown`, `clear`, `top`, `bottom`, `both`),
    `low_drop_db`, `high_drop_db` and `looks`;
  * `metrics.clipping`: the share over the last 10 s, and whether it is over the line.
  The panel is their only reader.
* A recording's sidecar keeps `passband.readings` in the reading's own columns, and
  `passband.start`/`end` hold its profile.
* `aetherd --passband-trace` runs the station's own reading over a recording.
* `tools/rxfilter_corpus.py` judges it on the corpus, the field and the harness, as Gate 3 did.

## 3. What it removes

* `spectrum.rs`: `PassbandMonitor`, `passband_edges`, `passband_width_hz`, `PassbandEdges`, their
  eight constants (`PASSBAND_DROP_DB`, `EDGE_RUN`, `FLOOR_QUANTILE`, `EDGE_HOLD_HZ`,
  `REFERENCE_HALF_HZ`, `MIN_OBSERVATIONS`, `HISTORY`, `NOISE_QUANTILE`) and their tests, which come
  back in the new reading's form.
* `station.rs`:
  * the wait for whole windows of quiet and the busy detector's attack time before a look
    (`passband_quiet_since`, `passband_pending`);
  * `PASSBAND_MIN_LEVEL_DBFS`, since the floor test refuses a silent input;
  * `rx_passband_hz`.
* The panel: `applyPassband`, its 400 Hz slack, and `#rx-passband-note` with its one sentence for
  every fault.
* The review's scaffolding in `crate::passband` and the trace:
  * the candidates R0–R2 and the variant dispatch;
  * `--passband-variant` and `--no-clip-veto`;
  * the trace's columns of today's estimate.

What it adds:
* the reading (about 210 lines, against today's about 150 in `spectrum.rs`);
* C1 (about 35);
* the banners and the Status line in the panel.

The code is about level with today's, with one constant fewer, and its tests cover the cases
today's never did: a moved passband, a cut low edge, a narrow filter, and a shallow chain floor.

## 4. What was measured

(`docs/research/receive-filter/04-results.md`)

* **The corpus**, two draws (130 clips each, with line and joined clips): 48 of 48 cuts of 20 dB
  or more named on the right side, no warning on any of the 160 clip-bandwidths with both edges
  10 dB or less down, and no warning naming a side not cut. Every cut on a steady band and on the
  FT8 band was named in 11.5 s. A cut taken away cleared 21–41 s after the knob, and a cut made
  warned 31–52 s after it.
* **The field**: Saturday's three 2300 Hz sessions named as top cuts at 11.5 s, 27.7–29.3 dB down.
  The two 500 Hz sessions read 7.5 and 6.6 dB at 1740 Hz, against the frames' 8. The gateway's
  FTDX10, with its own slope, reads 6–9 dB and is silent.
* **The harness**: 21 of 24 cuts of 20 dB or more named, and none of 138 uncut or ≤ 10 dB
  recordings warned. The three misses say *can't check*: the 500 Hz top cut over the shallow floor,
  where the called station kept only 10 or 11 looks clear of frames in its whole recording.
* **C1**: every hot corpus clip within 10 s and nothing else; Saturday's five within 0.02–4.1 s
  and none of the other fourteen recordings; all 24 hot harness runs and nothing else.
* **The cost**: inside the trace's own noise, about 0.03 % of real time.

## 5. The port (Gate 5)

1. `crate::passband` becomes the station's reading and C1. `Station::sample_passband` takes a look
   every half second while the station listens, holds it against the frames the receiver announces
   and reports, and drops the quiet-window wait. The deletions of §3 follow.
2. The control API's `metrics` and `docs/spec/control-api.md` change as §2.6 says.
3. The panel:
   * the two banners, shown in every view with their dismissal;
   * the Status tab's *Receive filter* line;
   * the Receive level tile's clipping line reading C1;
   * a tooltip on everything new.
4. The manual: the Status tab's line, the banners, and what to turn for each. The field-test
   guide's troubleshooting gets the same.
5. The tests, each today's in the new form:
   * the width as a span read at its edges;
   * flat noise;
   * the notch;
   * the FT8 band;
   * the wait for looks;
   * the looks kept from frames and keying.
   New tests cover a moved passband, a cut low edge, a narrow filter, a shallow floor, *can't
   check*, and clipping.
6. The checks before a beta:
   * the corpus and its held-out draw through the station's reading (K1, K2, K3, K5);
   * the field (K4);
   * the cut grid on the built daemon, with every run passing as before and no radiated overlap
     (K9);
   * the replay's cost on the mini PC's recordings, no more than 2 % of real time over today's.

## 6. Consequences

* The operator is told which knob, and a moved passband or a low cut set high is caught. A band of
  FT8 is read in seconds.
* The 500 Hz bottom edge warns from 12 dB down, where a cut costs nothing until about 40 dB. One
  line for every edge is simpler, and the author accepted the early warning there.
* A 20 dB top cut at 500 Hz over a shallow floor reads about 12 dB, on the line: the whole 500 Hz
  span sits on the filter's skirt.
* A station whose session starts the moment it starts listening has no reading for a while, at
  500 Hz possibly for the session. A station that has listened before has its reading in hand.
* Untested: a wide signal of a third station, such as an SSB contact, sitting on the frequency
  most of the time. The median over looks protects only while such a signal is there for under
  half of them.
