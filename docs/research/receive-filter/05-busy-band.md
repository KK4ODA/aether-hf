# The receive filter on a busy band: the false warnings of 2026-11-17

*Written 2026-11-17. The author saw the receive-filter banner (ADR-0105's reading, R3) warn at
both stations during the afternoon's RadioPost sessions on 7.101 MHz, though both radios' filters
were set right. This finds why from both stations' uploads, adds the missing case to the bench,
and registers candidate fixes and their criteria. **Nothing here changes the reader or the
panel.** No candidate has been built or run; R3 itself was run on the new bench case, to show that
the case reproduces the fault.*

## 1. What the files hold

The stations, 40 m at 2300 Hz on both ends, sessions 19:56–20:29Z:

* **The mobile**: `KK4ODA-2`, answering as `WD5EMA` and `KK4ODA`. A TS-480 on beta.161, which
  has no scan.
* **The gateway**: `KK4ODA-1`, an FTDX10 on beta.162 in DATA-U with AGC FAST. It scanned 7.101 and
  3.590 MHz from 17:12Z (ADR-0129). The scan stopped at 20:14:50Z, when the radio did not answer a
  dial change, and the gateway stayed on 7.101 MHz after that.

There are twelve uploads with sixteen recordings: nine from the gateway, seven from the mobile.
Each recording keeps the reading look by look (`passband.readings`, the live station's own).
Each upload's `diagnostics.json` holds the reading at the moment it was made.

**No warning was captured.** Every live reading in the sixteen recordings says *clear*, and so do
the twelve snapshots:

| Station | Live readings in its recordings (bottom / top drop, dB) | Snapshots at upload |
|---|---|---|
| Gateway | 3.8–9.9 / 1.4–11.3 | 5.5–7.5 / 0.8–10.4 |
| Mobile | 2.2–5.6 / 2.0–5.5 | 3.1–7.6 / 1.0–5.6 |

The daemon does not log the verdict. Recordings cover only sessions and the 30 s of audio before
each (the pre-roll). So the banners the author saw came while the stations idled between
sessions, and no file holds them. What the files do hold is the stations' idle audio in each
pre-roll, and the live readings drifting toward the line. That is enough to find the cause and
to reproduce the fault (§2), though not the author's exact moments.

## 2. The cause: other stations' signals that R3 cannot leave out

### 2.1 What the gateway heard

Every look of the gateway's 20:27:34Z pre-roll holds a signal about 400 Hz wide at 1100–1500 Hz.
In the median look it stands 7.3 dB over the look's span median, and 6 dB or more in 77 % of the
looks. Every six seconds it swells, and the FTDX10's AGC pulls the rest of the band down 12–15 dB
under it. The live reading in that session is 9.8 dB at the bottom and 11.0 dB at the top. On
clean stretches the same radio reads 2–4 dB at both edges, on both dials (the 19:56Z pre-roll's
dwells).

R3, run fresh over each gateway recording's own idle audio (`aetherd --passband-trace`), warns in
three of the nine:

| Recording | What is on the channel | R3 says |
|---|---|---|
| 20:03:14Z | A block at 650–950 Hz, on (6 dB or more) in 44 % of the looks. In a run of looks the AGC pulls the band 7–11 dB down under it | *both*, and *bottom* at the end, 12–16 dB, in 37 readings |
| 20:05:23Z | A signal over 1700–2600 Hz, the top 40 % of the span, for 25 s after the scan's hop onto 40 m | *bottom*, 12–13 dB, in 20 readings |
| 20:17:11Z | A block at 1000–1300 Hz, on in 40 % of the looks | *bottom* once, at the first reading (12 looks) |

**The mobile heard the same signals, more weakly.** At 20:02:45–20:03:00Z it heard the same
650–950 Hz block as the gateway did, on in 20 % of its looks. Its readings in sessions stayed at
2–6 dB. Its 20:07Z session ended at 2.2/2.0 dB. After 50 s of idle listening its snapshot read
7.6 dB at the bottom and 1.0 dB at the top: a signal over the top of the span raising the level,
with the bottom reading the difference. While idle, its busy detector logged "the passband is
peaked" every few seconds. No recording holds the mobile's warning. The corpus's clips over its
shallow chain floor warn under the same signals (§5), so this is inferred, not seen.

### 2.2 How R3 turns them into a cut

Rule by rule (ADR-0105 §2.1):

1. **The leave-out reaches only narrow signals.** A bin is left out when it stands 14 dB over the
   lower quartile of the 150 Hz on *each* side. A signal wider than about 300 Hz has signal on
   both sides of most of its bins, so it stays in. So does a group of signals close together:
   RTTY's mark and space are 170 Hz apart, and a contest packs four of them in.
2. **The look's level is its ninth decile over the span.** A signal covering more than a tenth of
   the span (228 Hz at 2300 Hz) sets it. The look is divided by the signal's level, so its noise
   sits that much lower, at both edges too.
3. **The profile is each bin's median over the looks.** A signal on in more than half the looks
   stays in the profile.
4. **The reading's level is the profile's ninth decile over the span.** That is the signal again,
   and each edge reads the signal's height over the noise as a drop.
5. **The AGC makes it deeper.** A strong signal pulls the band's noise down and stays itself. In
   those looks the edges read 20–29 dB down.

A block 5–7 dB over a look's median raised both edges' drops by about as much in the looks where
it was on, and by more where the AGC pulled the noise down under it. That took the FTDX10 from
2–4 dB to 9–15 dB. A radio's own slope (the FTDX10's) or a little roll-off at an edge (the
TS-480's 2–6 dB) leaves 6–10 dB to the line on a quiet band. On a busy band the signals take the
rest.

### 2.3 The harness shows it every night

Today's nightly (run 44093204950, 13:29Z, `c48e54ed`) keeps every station's live readings.
`40m-rtty-contest-2300` puts four RTTY stations inside the passband. On radios that cut nothing,
it warned in all three draws, at five of the six stations: *bottom* or *both*, 12–18.7 dB, in 162
of 931 readings.

The other scenarios with other stations stayed clear: the VARA-class ones, the PACTOR one, the
RTTY-and-crashes one and the 500 Hz ones. Their signals are on less of the time, sit outside the
span, or cover all of it.

The nightly does not judge the filter, so nobody saw this. Gate 5's harness check ran only the cut
grid (`tools/rxfilter_cost.py`), which has no other stations on the air.

## 3. What it is not

* **The scan.** Both of the gateway's dials read 2–4 dB, and the FTDX10's passband is the same
  shape on both. Until 20:14:50Z about half the looks came from 80 m, which does not carry the
  40 m signals, and that diluted them. After the scan stopped, every look was 40 m, and the
  gateway's readings rose from 7.9/4.4 dB at 20:17Z to 9.8/11.0 dB at 20:27Z. The mobile does not
  scan.
* **A small leak at a hop.** A hop that lands in a frame already under way restarts the receiver,
  which then never announces that frame, so R3 keeps its looks. In the 19:56Z pre-roll, the scan
  hopped back to 40 m at 25.1 s into a strong signal. The receiver announced only a tone frame from
  28.8 s, so the 7 looks before it were kept, reading the bottom 17.5 dB down. Most likely that was
  the first, OFDM part of the mobile's call: the mobile's log has it keying for 6.6 s from about
  then (the two stations' clocks differ by a few seconds). A handful of looks a call cannot move a
  median of 120. See S1 in §6.3.
* **Keying, the other station's key-up, and frames.** Sessions fold almost no looks: 9 in the
  gateway's 496 s session at 20:07Z, while the station is keyed, deaf or receiving. The reading
  comes from idle listening. Frames that are announced or reported are held out, as designed.
* **The radios and the line.** On clean stretches the FTDX10 reads 2–4 dB and the TS-480 2–6 dB.
  Neither radio is near the 12 dB line on its own.
* **A move between bandwidths** (`set_air`): neither station moved. **Clipping**: none (the clipping share C1 0.0 %).

## 4. Bug or judgement

**A judgement.** R3 does what ADR-0105 says. Its leave-out is built for narrow signals: a second
signal beside the first moves a side's noise only once it holds three quarters of it. ADR-0105 §6
lists this case as untested: "a wide signal of a third station … sitting on the frequency most of
the time. The median over looks protects only while such a signal is there for under half of
them." On 7.101 MHz on a weekday afternoon, such signals were there for a third to three-quarters
of the looks, and every few seconds, at both stations.

So the fix is the author's decision before anything is built. The bench missed the case three
ways:
* the corpus had only FT8, which is narrow;
* the cut grid had no other stations;
* the nightly's scenarios with other stations do not judge the filter.

## 5. The bench, added

**The corpus** (`tools/rxfilter_corpus.py`) gains a *qrm* kind: 72 clips of 90 s. Each has other
stations wider than the leave-out reaches, inside the span. They are modelled on what was heard:

* a 500 Hz multicarrier at 1300 Hz (20:27Z);
* a PACTOR-class pair at 800 Hz (20:03Z);
* a 2.4 kHz VARA-class signal over the top half (20:05Z);
* the harness's four RTTY stations.

Each comes on its own rhythm (on 70–90 % of the time) or steadily half the time. Each is
*moderate* (the 500 Hz ones stand 6.7–6.9 dB over a look's median, on 64–66 % of the looks: the
field's figures) or *strong* (8 dB more, with a fast AGC answering it).

The radios are three uncut ones, heard at both duties:
* a flat filter over the deep chain floor;
* the FTDX10-like slope over the deep floor;
* a flat filter over the mobile's shallow floor.

And three cut ones, heard at their own rhythm only: a top cut, a bottom cut and a narrow filter.
They are judged as the corpus clips are, and counted apart.

R3 on them (the beta.161 daemon; its reader is the tree's, unchanged since beta.145):

| | R3 |
|---|---|
| Uncut clip-bandwidths warned (K2) | **36 of 120**: moderate 5 of 60, strong 31 of 60; 25 of 48 at 2300 Hz, 11 of 72 at 500 Hz; the 500 Hz multicarrier 18 of 30 |
| Cuts of 20 dB or more named right by the end (K1) | 22 of 24. The two misses read *both*: the signal adds the side not cut |
| Warnings naming a side not cut (K3) | 41 clip-bandwidths: 36 uncut, 5 cut (all strong, always on) |

Four of the five moderate clips that warn are the gateway's case: the FTDX10-like slope under the
500 Hz multicarrier or the PACTOR pair. The two on their own rhythm end on *top* at 12.5 and
13.3 dB, the slope's edge plus the signal's height. The fifth is the top-cut radio heard at
500 Hz, whose 1740 Hz edge is only 6.8 dB down, ending on *top* at 14.1 dB under the multicarrier.

**The field** set gains today's sixteen recordings as working stations (K2). A recording keeps its
events from its start only, so a caller's own call tries in the pre-roll were unmarked. The trace
took them as looks, and the TS-480 mutes 1–2.8 kHz while keyed, so they read the top 20–30 dB down.
A live station takes no look while keyed. The field judge now adds the keying the upload's
`aetherd.log` holds for the pre-roll (`preroll_keying`). R3 traced on the sixteen warns on three:
the gateway's 20:03:14Z, 20:05:23Z and 20:17:11Z (§2.1).

**The harness** needs no new scenario. Its scenarios with other stations, judged on their live
readings, are the case (§2.3). H1 in §6.3 would make the nightly say so.

## 6. Candidates, registered before any run

What separates a third station's signal from the radio's filter is **time**. The filter never
changes, while a signal starts and stops (an ARQ exchange's bursts, an RTTY over, a QSO's turns)
and the AGC moves with it. Frequency cannot separate them: a 2.4 kHz signal over the bottom 60 %
of the span has the same spectrum as a passband moved down by an IF shift. Neither can depth: a
narrow filter over a shallow floor is a 15 dB hump, the same as a 15 dB signal.

The names follow the review's: R4 is the reading after R3, and W2 the witness after W1. C1 stays
the clipping share of ADR-0105 §2.2.

### 6.1 R4: a signal comes and goes, the filter stays

1. **Each look is divided by the median** of its span's bins left in (today: their ninth decile).
   A signal on part of the span no longer sets the look's level, so the look's noise, the edges
   included, stays where it is. The floor test still uses the ninth decile, unchanged.
2. **Each bin's profile value is its 20th percentile** over the looks it was free in (today: the
   median). A signal that is on in up to 80 % of the looks no longer sets it.

Everything else stays as it is:
* the leave-out;
* the hold;
* the 120 looks;
* the reading's ninth decile over the smoothed profile;
* the 12 dB line.

* **Removes**: the per-look ninth-decile division, which turned a signal on part of the span into a
  lower noise everywhere else, and the median over the looks.
* **Adds**: one constant, the quantile over the looks (0.2), for twelve against R3's eleven. Why
  0.2: the field's signals were on in 33–77 % of the looks, so it must be under 0.23. It must also
  stay well above 1/120, so that a few looks deep in a fade do not set it.
* **Costs**:
  * A signal on more than about 80 % of the window still reads as a cut. Two stations of an ARQ
    exchange heard at once can be on that long. The warning then clears when they stop.
  * A cut taken away clears once 80 % of the window is new: about 48 s of listening and the 5.9 s
    hold. Today it takes 21–41 s (K5 asks for 60). A cut made warns once 20 % is new, which is
    sooner than today.
  * A low quantile has fewer looks under it, so the edges read a little noisier. The ±50 Hz
    smoothing is there for that.
* **Ablations**: R4a is the median division alone, R4b the quantile over looks alone. A part
  whose removal fails no criterion goes.

### 6.2 W2: the frames clear an edge

W2 is the only rule that can clear a signal that is always on. A warning is not shown for an edge
when the other station's OFDM frames, decoded in the last N minutes at the same bandwidth, show
that edge passing within 10 dB of the middle. Each frame's channel estimate, carrier by carrier,
is the transmit filter, the path and this radio's receive filter together. If the edge carriers
arrive whole, this radio does not cut that edge 20 dB.

* **Removes**: nothing.
* **Adds**: the per-carrier reading of decoded frames in the daemon, which Gate 3 built as W1 and
  Gate 5 deleted (`tools/passband_witness.py` still has it), and a memory of the last one: about
  100 lines and two constants (N, and the 10 dB).
* **Costs**:
  * It only acts after frames: an idle warning before the day's first session stays. Today's
    warnings came while idle.
  * It reverses the author's Gate 2 answer on the frames' role: there, frames could only bring a
    warning forward; here, they veto one.
* **Recommendation: R4 first.** W2 only if the author wants the always-on case closed and accepts
  the frames as a veto.

### 6.3 Instruments and the scan's leak (each the author's go)

* **L1, the verdict in the log.** One line in `aetherd.log` each time the verdict changes, with
  both drops and the looks, in the form of the busy detector's transitions:
  `receive filter: clear -> top | bottom 5.1 dB, top 12.4 dB, 120 looks`. Today's warnings could not
  be placed because nothing logs them. About ten lines and no constant. It changes the daemon,
  not the reader.
* **H1, the nightly reports it.** `session_matrix.py` lists a warning on a station whose radio has
  no `rx_filter_hz`, as a line in the nightly's report. It only reports, and fails nothing until a
  fix is in. With the fix, it becomes an invariant.
* **S1, a call the scan lands in partway.** For the reader, a frame announced within the air's
  longest frame after a hop starts at the hop, because the try's first part was never announced.
  About five lines. Register it only if R4's runs show those looks matter. On today's evidence they
  do not: 7 looks a call, which a 20th percentile over 120 absorbs.

### 6.4 Considered and not registered

* **A higher line.** A 20 dB cut over the mobile's shallow floor reads 13–14 dB (ADR-0105 §2.1), so
  a higher line misses it. The signals took the drops to 13–18.7 dB anyway.
* **A longer window.** A low quantile over several minutes clears more signals, but a cut taken
  away would clear minutes late (K5).
* **Looking only while the busy detector calls the channel clear.** A 400 Hz block 7–10 dB over the
  noise raises the 2280 Hz channel's power 2–4 dB, at or under the detector's margin (4 dB at the
  gateway, 6 at the mobile). A band of FT8 would make
  the reading wait minutes again, the estimate's old fault. And the busy detector is at its last
  patch.
* **A wider leave-out.** Sides wide enough to take a 500 Hz signal would also take a 500 Hz filter's
  passband under the 2300 Hz span, and the narrow filter would go unnamed. Signals of 2.4 kHz are
  out of reach of any side test.

## 7. Criteria, registered now

Judged with the reading built into a daemon, on the corpus (draws 0 and 1, `--seed 1`), the field
and the harness. A narrow miss goes to the author with its trace, case by case; it does not pass by
rule.

| | Criterion | R3 today |
|---|---|---|
| Q1 | **Silence under other stations.** No warning on any qrm clip over an uncut radio, at either bandwidth, for the moderate clips (own rhythm and half time) and the strong half-time ones: 84 clip-bandwidths. The strong clips on their own rhythm (on 74–86 % of the time under a fast AGC: 36) are counted and reported, not judged (§6) | 18 of 84 warn; 18 of 36 reported |
| Q2 | **Detection under other stations.** Every qrm clip over a cut radio named on the right side by the end (24), with no side not cut named on the moderate ones (12). On the strong ones an extra side is reported, not judged | 22 of 24; moderate 0 extra; strong 5 |
| K1–K3, K5, K6 | **The corpus as at Gate 5, both draws.** 48 of 48 cuts named; 0 of 160 uncut warned; no side not cut named; 24 of 24 in time; joined clips 12 of 12 cleared within 60 s and 12 of 12 warned of a cut made; C1's clipping verdicts unchanged | met |
| K2 field | **No warning on a working station.** None of today's sixteen (with their pre-roll keying) and none of the fourteen of Gate 3; the 2026-11-07 15:06Z recording stays in the free band | today's: 3 of 16 warn |
| K4 | **Saturday**, as at Gate 5: the top named in the three 2300 Hz sessions; the 500 Hz edge within 3 dB | met |
| H | **The harness.** (a) The nightly's scenarios with other stations (`40m-rtty-contest-2300`, `40m-moderate-2300-vara-qrm`, `40m-hidden-vara-2300`, `40m-good-2300-rtty-crashes`, `80m-vara-on-frequency-500`, `80m-poor-500-pactor-signalink`, `80m-nvis-evening-500`, `80m-disconnect-slow-drain-500`), three draws: no warning at any station. (b) The cut grid as at Gate 5: 20 of 24 cuts named (the misses *can't check*), 0 of 138 uncut warned, every run passing with no radiated overlap | (a) 5 of 6 stations warn in `40m-rtty-contest-2300`; (b) met |
| K7 | At most one constant more than R3's eleven; says what it removes; code about level | — |
| K9 | The trace's cost within 2 % of real time on the mini PC's recordings | — |

## 8. For the author

1. **Build R4?** It clears the signals that come and go: every warning the files show today came
   from signals on a third to three-quarters of the time. The cost is a reading that clears about
   48 s after the knob instead of 21–41 s, and one constant more.
2. **The always-on case.** A wide signal on the frequency for most of a minute will still read as a
   cut under R4, and it clears when the signal stops. Accept it, or add W2 (the frames as a veto)?
3. **L1 and H1 now?** Neither changes what the reader says. L1 lets the next field report place a
   warning. H1 shows the harness's warnings every night, as `40m-rtty-contest-2300` has been
   giving them.
4. **The mobile's own warning** was not captured (§2.1). If it comes again before a fix, a recording
   made while idle (*Record* on the Session tab) with the band busy would show it, and L1 would date
   it.

**The author's answer** (relayed by the session that started this one, 2026-11-17): "fix all
items", the filter alert's false positives included, the Rust work to a cloud session. §10 is the
build, read as R4 (§6.1), L1 and H1; S1 only if its check shows the leak matters; and W2 only on the
author's explicit word, since it reverses the Gate 2 answer on what the frames may do.

## 10. The build

For a session with a Rust toolchain. The criteria of §7 are fixed; nothing below changes them. The
frames' witness agrees that neither radio cut: `tools/passband_witness.py` on the gateway's 20:07Z
recording reads the mobile's 322 OFDM frames −1.1 dB at 300 Hz and −6.1 dB at 2600 Hz, and on the
mobile's side the gateway's 59 frames within 3 dB from 300 to 2600 Hz. Every warning of that
afternoon was false.

### Step 1: the trace follows the station's hops

Changes:
* **`core/aetherd/src/passband_trace.rs`.** At each scan hop the sidecar holds
  (`Expectation::restarts`, pairs of `[t, t + deaf]`), hand the reading `filter.frame(t, t + deaf)`,
  as `station/scan.rs` does at a hop (`self.passband.frame(now, now + deaf)`). Today the trace
  ignores hops.
* **`tools/rxfilter_corpus.py`, `preroll_keying`.** Also add the pre-roll's hops from the log's
  `dial: moved to <Hz> Hz` lines, as `hop` events with the detail `dial <Hz> deaf 0.500` (the form
  `replay::hop_deaf_s` reads). A recording keeps no events before its start.

Check: the trace prints the same lines as before over every field recording without a hop, and
differs only around hops (the gateway's pre-rolls before 20:14:50Z).

### Step 2: R4, in `core/aetherd/src/passband.rs`

1. **A new constant**, after `REFERENCE_QUANTILE`:
   ```rust
   /// Where in each bin's values over the looks kept the profile is read: the fifth. Another
   /// station's signal comes and goes and the radio's filter stays, so a bin's low values are the
   /// filter's: a signal on in up to four looks of five no longer sets it. On 7.101 MHz on
   /// 2026-11-17 the signals the leave-out could not reach were on in 33-77 % of the looks
   /// (`docs/research/receive-filter/05-busy-band.md`).
   pub const TIME_QUANTILE: f64 = 0.2;
   ```
2. **`ReceiveFilter::fold`.** Keep the ninth decile (`level`) for the floor test, as now: it sets
   `last_over` and counts a look `weak`. Divide the look by its median over the span instead:
   `quantile(&mut bins_between(&look, bin_hz, span.low_hz, span.high_hz), 0.5)`. A signal on part
   of the span no longer sets the look's level.
3. **`ReceiveFilter::profile`.** `quantile(&mut column, 0.5)` becomes
   `quantile(&mut column, TIME_QUANTILE)`. The rule that a bin a signal held in more than half the
   looks is NaN stays.
4. **`ReceiveFilter::set_air`.** Divide each kept look again by its median over the new span, not
   its `REFERENCE_QUANTILE`. The looks must stay divided the way `fold` divides them.
5. **Unchanged:**
   * `span_drops`, which reads the ninth decile over the smoothed profile;
   * `verdict_of` and `LINE_DB`;
   * `leave_out_occupied`;
   * the hold, `LOOKS_NEEDED` and `LOOKS_KEPT`;
   * C1, the clipping share.
6. **Words.**
   * The module's doc comment: a look is divided "by its median over the signal's span", and the
     profile is "each bin's fifth over the looks kept".
   * `record.rs`'s `passband_profile` doc: the values are relative to the span's median, where
     they were relative to the passband's level. The sidecar's form does not change.
7. **Tests.** The nineteen in `passband.rs` must pass with what they assert unchanged (verdicts
   and drops). Two new ones, each seen to fail on R3's rules first:
   * `a_wide_signal_on_part_of_the_time_is_not_a_cut`: flat noise, as in
     `flat_noise_passes_the_signal`, with a 400 Hz block 10 dB over the noise at 1100-1500 Hz in
     70 % of the looks, and the noise 10 dB down in those looks (the AGC). It reads `Clear`.
   * `a_cut_under_a_wide_signal_is_still_named`: the moved passband of
     `a_passband_moved_down_reads_as_its_top_cut`, with the same block. It reads `Top`.

   `a_change_of_bandwidth_keeps_the_looks_and_judges_the_new_span` covers `set_air`.
8. **Constants:** twelve, R3's eleven and `TIME_QUANTILE`. The look's median is a median, not a
   setting.

### Step 3: L1, in `core/aetherd/src/station.rs`

* A field on `Station`: `filter_said: Verdict`, starting `Waiting`.
* In `sample_passband`, after `step`, read `self.passband.reading()`. When its verdict differs
  from `filter_said`, note one line and remember the verdict:
  `self.note("filter", "<old> -> <new> | bottom <x> dB, top <y> dB, <n> looks")`, with the verdicts'
  `word()`, the drops to a tenth of a dB, and `-` for a drop the reading lacks. It reaches
  `aetherd.log` and the panel's Log as every note does.
* A test: a station fed a cut and then the same radio uncut notes `waiting -> top` and then
  `top -> clear`, and nothing for readings that do not change.

### Step 4: H1, in `tools/session_matrix.py` and `tools/nightly_report.py`

* `filter_warnings(run)` takes each station whose scenario gives its radio no
  `[stations.<who>.radio] rx_filter_hz`, and reads its recordings' `passband.readings`
  (`<run>/<who>/recordings/*.json`). For each recording with a reading whose verdict is `top`,
  `bottom` or `both`, it returns `"<who>: <verdict> up to <drop> dB in <n> of <m> readings
  (<recording>)"`. `scratch/nightly_filter.py` is the same reading over a nightly's artifacts.
* `judge()` puts them in the result as `filter_warnings` and the CSV gains the column. They are not
  in `invariants` yet.
* `nightly_report.py` lists them among the night's findings, as a warning on a green run.
* **When they become an invariant:** once step 6 shows none on three draws of H(a), they move into
  `invariants`, in their own commit. A narrow miss there goes to the author first.

### Step 5: S1, only if it matters

First measure it. On the field set, with step 1's hops in the trace, count the looks R4 keeps
between a hop and a frame announced within the air's longest frame after it. The gateway's 19:56Z
pre-roll has 7. If leaving those looks out changes no verdict on the field set or the harness, S1
is not built, and the decision record says so.

If it does matter:
* `ReceiveFilter::hop(at)` remembers the last hop.
* `frame(start, end)` takes `start = last_hop` when `0 <= start - last_hop <= hold_s - 0.5` (the
  air's longest frame).
* `scan.rs` calls `hop(now)` beside its `frame(now, now + deaf)`, and the trace calls it at each
  restart.

### Step 6: judge it (§7)

* **Corpus.** `python tools/rxfilter_corpus.py make OUT`, and `make OUT1 --seed 1` (every clip,
  the qrm kind with them). Then `judge OUT --daemon core/target/release/aetherd` and the same on
  `OUT1`. K1–K3, K5 and K6 come from the first line, Q1 and Q2 from the `qrm:` line.
* **Ablations**, two scratch builds judged the same way. A part whose removal fails no criterion
  goes.
  * R4a: the median division alone (`TIME_QUANTILE = 0.5`).
  * R4b: the fifth over the looks alone (the look divided by `REFERENCE_QUANTILE` as before).
* **Field.** `python tools/rxfilter_corpus.py field field/inbox --daemon …` over the 2026-11-17
  uploads, and over Gate 3's nineteen (`tools/fetch_uploads.py get 20261106`, `20261107`,
  `20261108`, one at a time). K2: none of the thirty working recordings warns. K4: 5 of 5.
* **Harness (a).** The scenarios of §7's H(a), three draws each, on the built daemon:
  `session_matrix.py --seeds 3 --keep-passing` with H1's list. No warning at any station.
* **Harness (b).** The cut grid: `tools/rxfilter_cost.py OUT --keep-audio`, then
  `rxfilter_corpus.py harness OUT`. Every run passes as before, with no radiated overlap.
* **Cost (K9).** `aetherd --passband-trace` over the mini PC's longest field recording, best of
  three, within 2 % of real time.
* A miss goes to the author with its trace. Nothing passes by rule.

### Step 7: the record and the manual

If the criteria hold, it is recorded in an ADR amending ADR-0105: §2.1 steps 3–4, and §6's
untested case, now measured, with what R4 removes and its one constant. The next free number is
ADR-0130. It ships with:
* `docs/manual/manual.md`, in the same commit. One sentence under *The receive filter banner says…*:
  a strong signal from another station sitting on the frequency for most of a minute can read as a
  cut, and the banner goes when the signal stops. A change-log row with the header's Version and
  Updated. *The reading follows the knob within about a minute* stays true (about 54 s).
* `CLAUDE.md`'s receive-filter paragraph.
* A beta, by the standing rule.

If W2 is wanted after that, it is a step of its own, specified in §6.2. It is built only on the
author's explicit word, since it reverses the Gate 2 answer.

## 9. Method and scripts

`scratch/` keeps the scripts behind the numbers above (its README). The main one, `r3.py`, is a
numpy mirror of R3: on a corpus clip it reproduces the daemon's trace to 0.005 dB. Around it:
* `looks.py` gives each look's own drops, its sub-band levels and where it sits;
* `segments.py` gives each stretch's mean spectrum and its own reading;
* `duty.py` measures how much of the time a block was on;
* `summary.py` lists every recording's live readings;
* `nightly_filter.py` reads every harness station's live readings from a nightly's artifacts.

The uploads are in `field/inbox/` (git-ignored); the scripts read them with `python -I`. There is
no Rust toolchain on the machine this was written on. The traces ran on beta.161's daemon, whose
reader is the tree's (`git log core/aetherd/src/passband.rs`: unchanged since `05a9c17`, Gate 5).
