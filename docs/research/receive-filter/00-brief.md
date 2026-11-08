# The receive-filter estimate's review

*Started 2026-11-08 at the author's request ("start the review of the receive-filter
estimate"). The estimate has had its third fault, and CLAUDE.md's rule for a third fault is a
review, not a patch. The method is the one the link, the busy detector and the rate controller
were reviewed by: say the job in plain words; build instruments and gather evidence first;
register candidates and criteria before any run; build prototypes beside the shipping code; let
the author decide at each gate; and end with a decision ADR that says what it removes.*

## 1. Why a review

The estimate is `PassbandMonitor` and `passband_width_hz` in `core/aetherd/src/spectrum.rs`,
fed by `Station::sample_passband` in `station.rs`, reported as `metrics.rx_passband_hz` and
shown by the panel's `applyPassband` (`app/ui/app.js`) as a note at the top of the Status tab.
It changes nothing the modem does. It only warns.

### 1.1 What it is for

It came from the first ten-mile test (2026-09-23, `91ba8fb`): a radio left on its 500 Hz filter
while the modem ran 2300 Hz carried only the middle of every burst, and a whole on-air test was
lost before anyone noticed. Asking the radio over CAT was ruled out then: Yaesu's `SH` answers
an index that depends on the model and the mode, only Hamlib turns it into hertz, and it misses
an IF shift, a notch and the wrong mode. Band noise fills the whole passband, the filter shapes
it, and the shape is in the audio whatever the radio. So the estimate reads the passband from
the noise between signals.

### 1.2 Its three faults

| # | When | What it said | What was true | The fix then |
|---|---|---|---|---|
| 1 | 2026-09-26 (`d1b0992`) | "about 650 Hz wide" on the bench pair at 12 dB | flat noise to 4 kHz: it had taken the start of the other station's tone frames for noise | look only after a window of quiet, keep a look only if the quiet holds for the busy detector's attack time |
| 2 | 2026-11-05 (`7ac47c6`, beta.131) | "about 850 Hz wide" on 7.074 MHz | a 3 kHz filter: the average of the looks sloped down from a middle full of FT8 | each bin's 10th percentile over the last 120 looks; the edge 6 dB under the middle or halfway to the floor, whichever is lower; an edge must stay down for 150 Hz |
| 3 | 2026-11-08 | 1957–1980 Hz wide, so no warning (it shows below 2280 − 400 = 1880 Hz) | the mobile's TS-480 with its IF SHIFT turned fully left: the receive audio fell from about 1.5 kHz, 10 dB down by 1.7 kHz and 20–30 dB down from 2 kHz, where the 2300 Hz signal spans 360–2640 Hz | none: this review |

Each fix was right about the case in front of it and made the next case possible: the second
fix drew the edge "halfway to the floor", and on Saturday that put it 15 dB down the skirt,
over the warning line (§3 and `01-evidence.md`).

### 1.3 What the third fault cost

Two of Saturday night's sessions between the mobile and the gateway ran at 2300 Hz with the top
of every gateway transmission 20–30 dB down at the mobile. The path looked lopsided (the
gateway heard the mobile well, the mobile heard the gateway poorly), and the rate controller
paid for it with long waits on stuck frames. The operator found the knob the next day by
looking at the waterfall. Even the 500 Hz sessions lost something: the top tone of the four-tone
kinds, at 1650 Hz, was 4–7 dB down.

## 2. The job in plain words

> Tell the operator, while it matters and in words they can act on, when the radio's receive
> audio does not carry the whole of the modem's signal: a filter too narrow, a passband moved by
> an IF shift or a DSP cut, or a roll-off in the audio chain. Say which side is cut and by
> about how much. Never warn about a station that works: a band full of FT8, a session under
> way, an AGC at work, a fading band.

What this changes from the job the estimate was built for:

* **The span, not the width.** The signal occupies a band around 1500 Hz: 360–2640 Hz at
  2300 Hz, 1260–1740 Hz at 500 Hz. A passband can be wide enough and still miss one side of it.
  Saturday's was about 1.5 kHz wide by any measure, but a 2.4 kHz filter shifted 600 Hz down
  would be just as wrong and pass today's test.
* **Which side.** "Widen the radio's filter, or set the modem to 500 Hz" is the wrong advice for
  a shifted passband. The words must follow the cause: the top cut (IF shift, a high cut), the
  bottom cut (a low cut), both (a narrow filter).
* **No reference at the modem's centre.** Today's thresholds are drawn from the noise at
  1500 Hz. When the passband is shifted, 1500 Hz is on its skirt.

## 3. What is known before Gate 0

From `aetherd --passband-trace` (Gate 0 item 1, built first) over the 19 recordings in
`field/inbox/` (2026-11-06 … 11-08, the gateway's FTDX10 and the mobile's TS-480);
`01-evidence.md` has the tables. A first look with a Python mirror blamed the 10th percentile and
the noise level's swing; the real look selection does not bear that out, and it is withdrawn.

1. **The trace reproduces the live readings**: the 00:54Z session ends at 1980 Hz, the reading
   the mobile's daemon uploaded at 01:00.
2. **The estimate saw the cut.** On the mobile's clean looks the noise was flat to 1.4 kHz, 16 dB
   down at 1.8 kHz and 23 dB down from 2 kHz.
3. **The rule for the edge's depth put it in the wrong place.** "6 dB under the middle or halfway
   to the floor, whichever is lower" drew the edge 15 dB down the skirt, at 1.93–2.06 kHz, over
   the warning line (1880 Hz); at 6 dB down it falls near 1.65 kHz. The depth follows the audio
   chain's own floor: 6.5–16 dB on the mobile, 25–28 dB on the FTDX10. On clean noise through an
   eighth-order filter it reads 300–2700 Hz as 211–3176 Hz. The original claim that the estimate
   "can only ever read narrower than the truth, never wider" (`91ba8fb`) does not hold.
4. **The test is the width.** Even with the edge at 6 dB down, the panel would have said "too
   narrow, widen the filter" for a passband that was moved, not narrow.
5. **The reference at 1500 Hz** sat 1–7 dB under the band below it on Saturday.
6. **The low edge is never found.** Every reading runs to 0–23 Hz at the bottom.
7. **Two sessions are unexplained**: their looks' level swung 28–32 dB and their profiles slope
   26–29 dB with no clear edge. Every Saturday session clipped alike (1.3–1.7 % of samples at
   full scale), so clipping alone does not explain them.
8. **Where it shows.** The note sits at the top of the Status tab. It said nothing on Saturday,
   but had it spoken, the operator was in a session and may not have been on that tab.

## 4. Scope

* **In:** what is looked at and when (`sample_passband`), the noise profile, how edges are drawn,
  what is compared with what (span against width), what `metrics` reports, the panel's words and
  where they show, the manual's text, and what a recording keeps of it.
* **In, by the author's answer (§6.5): overdriven receive audio.** Telling the operator, during
  sessions as well as in Setup, when the receive audio is driven hot enough to clip. Setup step
  3's level check is an input to that work, and it may change.
* **In, as a second witness (§6.3):** the other station's decoded frames, whose pilots measure the
  passband across the signal.
* **Out, inputs only:** the busy detector (its own rule: the next fault is a review), which
  decides what is quiet; the radio's AGC.
* **Out (§6.4):** reading the radio's filter over CAT.

## 5. Gates

### Gate 0: instruments and evidence (`01-evidence.md`)

1. **The real code over a recording**: `aetherd --passband-trace WAV [--expect SIDECAR]`, every
   look and every reading as CSV, as `--busy-trace` does for the busy detector, so the evidence
   stops resting on a mirror. *Built 2026-11-08 (`passband_trace.rs`); §3 is its first result.*
2. **Recordings keep the estimate**: a sidecar's `passband` with the readings (both edges, not
   just the width) on a beat, so the next field case can be read after the fact.
3. **A labelled corpus** (`tools/rxfilter_corpus.py`): the field recordings in hand with what
   each radio was set to where it is known, and synthetic cases from the channel server's noise
   through filters that are narrow, shifted either way, notched and sloped, under AGC pumping,
   QSB, FT8, a session's traffic and clipping.
4. **Today's estimate over the corpus**: how often it is wrong, which way, and why.
5. **The count**: today's constants (eleven in the daemon and one in the panel), the lines of
   code, the tests.
6. **What a cut costs** (§6.2): the harness with the outer part of the signal cut by 3, 6, 10 and
   20 dB, each side and both, at each bandwidth, against the same draws uncut. The depth where the
   cost starts to show is the warning's threshold in Gate 2.
7. **Clipping** (§6.5): how often and how hard the field recordings in hand clip, what Setup step
   3 said of each station's level, and what clipping costs on the harness.

### Gate 1: a short survey (`02-survey.md`)

How noise power spectra are estimated under level swings and intermittent signals (minimum
statistics, quantile methods, normalising each look), how passband edges are measured in audio
test practice, and what other amateur modems tell the operator about the radio's filter.

### Gate 2: candidates and criteria, registered before the runs (`03-candidates.md`)

Candidates to weigh, at least: a profile that survives level swings (each look divided by its
own level before the percentile, or a mean over the looks the busy detector trusts most), edges
read against the passband's own level rather than the modem's centre, the span test with the
side named, the decoded frames' channel estimate as a second witness, and a clipping warning
that holds in a session. Criteria will ask for detection of a cut at either side of the span at the
depth where the harness shows it costs throughput (§6.2), no warning on the corpus's working
stations, words that name the side, and fewer constants than today.

### Gates 3–5

A prototype beside the shipping code, judged on the corpus and the harness; the decision ADR
with what is removed; the port, the manual, and the deletion of what it replaces.

## 6. For the author

1. **Field builds.** The estimate only warns, so nothing here needs field builds to hold still.
   Shall other work keep shipping as betas meanwhile?
2. **How much is too much.** At what loss at the signal's edge should the panel warn? A
   proposal for Gate 2: measure on the harness what a cut costs at each bandwidth (the outer
   carriers 3, 6, 10, 20 dB down) and warn where the cost starts to show.
3. **The frames as a probe.** May the review use the other station's decoded frames, whose pilots
   measure the passband across the signal directly, as a second witness beside the noise?
4. **CAT.** Reading the filter's settings over CAT was ruled out in September. Leave it out, or
   look again (the FTDX10's `SH0` is already read into recordings)?
5. **The input level.** The mobile's audio was at −4 to −12 dBFS and clipping on Saturday. That
   belongs to Setup step 3's level check, not here, but the estimate must not be fooled by it.
   Agreed?

### The author's answers (2026-11-08)

1. **Other work keeps shipping** as betas while the review runs.
2. **Measure the threshold.** On the harness, cut the outer part of the signal by 3, 6, 10 and
   20 dB at each bandwidth, measure what each costs, and warn where the cost starts to show.
   This joins Gate 0.
3. **Yes, the frames are a witness.** Candidates may use the decoded frames' channel estimate
   beside the noise; the criteria judge whether it earns its place.
4. **CAT stays out.** The audio is the one witness every radio has.
5. **Overdriven receive audio is in scope**: the review also looks at warning the operator about
   clipping receive audio during sessions, not only in Setup (§4).
