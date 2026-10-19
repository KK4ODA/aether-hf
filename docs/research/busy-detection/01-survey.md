# How busy detection is done: a survey

**The question.** Aether's busy detector learns a noise floor from its channel's own block powers.
A strong signal anywhere in the radio's IF passband makes the AGC hold the gain down, the noise in
the channel drops 10–20 dB and stays steady, and the detector learns that as the floor. When the
AGC lets go, the true noise reads busy for the floor window plus the hold, 71 s
(`00-recording-plan.md`, and the logs in `02-measurements.md`). This survey asks how other systems
decide that a channel is busy, and whether their way survives an AGC.

For each system or method it answers seven questions:

1. What quantity is detected?
2. How is the noise reference estimated, and how fast does it track up and down?
3. How are AGC and gain changes handled?
4. What is the decision logic (attack, hangover, hysteresis, dwell)?
5. What is known about its performance?
6. What does it cost?
7. Does it apply to an audio-coupled SSB transceiver with its own AGC, Aether's case?

**How to read the citations.** Each source is marked with how it was read: **[full]** the text
itself; **[partial]** the text, with some equations or numbers lost in extraction; **[abstract]**
the abstract or a record only; **[secondary]** a statement about the work in another source;
**[source]** source code read for its algorithm. GPL code was read for the algorithm only; nothing
from it is in this tree. VARA HF is covered from published documentation and observed behaviour
only. Where a number was worked out here rather than read, it says **(derived)**; where a
conclusion is ours, it says **(inference)**. The four working notes this summarises, with every
URL, page and commit, were kept with the research files of this investigation.

---

## 1. Military and commercial HF

### 1.1 MIL-STD-188-141 (B/C/D): 2G ALE

Sources: MIL-STD-188-141B (1999), 141C (2011), 141D (2017), everyspec.com **[full]**.

1. **Quantity.** Specific waveforms. Before calling or sounding, an ALE controller "shall listen
   for a programmable time (Twt) for other traffic" (141C/D A.5.4.7), detecting it "using any
   combination of internal signal detection and external devices that provide a channel busy
   signal" (A.5.4.7.2).
2. **Noise reference.** Not specified. The requirement is a test: Table A-I gives detection
   probabilities at fixed SNR (ALE, 110 serial tone, STANAG 4285/4529: 0 dB → 80 %, 6 dB → 99 %;
   SSB voice: 6 dB → 80 %, 9 dB → 99 %) with false alarms ≤ 1 %, on stationary AWGN, 2 s dwells.
3. **AGC.** Out of the detector's test entirely: Figure A-2 feeds baseband audio from a channel
   simulator straight into the controller, **with no radio and so no radio AGC in the loop**. The
   standard handles AGC at the radio instead (§1.2).
4. **Decision.** LBT of at least 2 s on general channels, 2 Trw (784 ms) on ALE-only channels;
   no hysteresis or hangover is specified. An occupied channel is skipped and revisited later
   (A.5.5.2.3.2). The operator "shall be able to override" both the pause and the lockout
   (A.5.4.7.3).
5. **Performance.** Only the required minima above; no published measurements of fielded
   detectors were found.
6. **Cost.** One detector per listed waveform plus a voice detector, run during 0.6–2 s dwells.
7. **Applies?** The requirement levels are a useful yardstick. A waveform recogniser normalised by
   its own statistics does not move with a gain step (inference), but it is blind to anything it
   does not recognise.

### 1.2 The radio's own AGC in the same standard

141B/C main body §5.4.3 **[full]**: in data service "the receiver AGC attack time shall not exceed
10 ms. The AGC release time shall not exceed 25 ms" (5.4.3.3); voice and ICW release in 800–1200 ms.
A 141-compliant radio in data mode cannot hold the noise down for seconds. MIL-STD-188-110D
App. C §C.7 **[full]** recommends the opposite for its QAM waveforms, a slow AGC, because "AGC, and
ALC can cause significant performance degradation"; the channel memory keeps an AGC setting per
channel (141 A.4.3.1, a design objective). **The military answer to our failure is at the radio.**
An amateur transceiver gives no such guarantee: the FTDX10's DATA-U default is MID, 500 ms of
decay after a hold (§3.8).

### 1.3 3G ALE (141B Appendix C) and STANAG 4538

STANAG 4538 is not public; 141C calls its earlier 3G appendix "essentially identical", so 141B
Appendix C **[full]** stands in for it. The FFI test report of STANAG 4538 in the Harris RF-5800H
**[full]** says nothing on occupancy.

1. **Quantity.** Waveforms again: Table C-II requires 3G-ALE (BW0) at −9 dB → 50 %, −6 dB → 95 %;
   2G-ALE 0 dB → 50 %, 6 dB → 90 %; SSB voice 6 dB → 50 %, 9 dB → 75 %; PSK serial tone 0 dB → 30 %.
   Below 0 dB in 3 kHz this is a preamble correlator, not an energy detector (inference: the
   energy detector's SNR wall, §3.1).
2. **Noise reference.** None stated. False alarms < 1 % "when it carries only additive white
   gaussian noise" (C.4.6.1.2), tested at audio as in 2G.
3. **AGC.** On the transmit side: every 3G burst starts with a "TLC/AGC guard sequence" so that the
   receiving radio's AGC settles before the preamble (C.5.1.3.1).
4. **Decision.** Synchronous mode listens for "at least 640 ms" in slot 0 of each dwell;
   asynchronous mode listens 2 s by default before calling, and the responder about 640 ms before
   answering. Occupancy is stored per channel with its measurement time. Inside a link, Circuit
   Link Control is "a simple listen-before-transmit access control": ModemDetect (110 serial tone,
   "including both the preamble and data portions") and AudioDetect (SSB voice, required); a
   suggested 800 ms reacquisition hangover; then a backoff drawn by priority from
   {0, 250, 450, 650, 850} ms (Table C-LXVII).
5. **Performance.** Required minima only.
6. **Cost.** Waveform-specific detectors and a voice detector.
7. **Applies?** The structure — recognisers for known waveforms, a voice detector, a short
   hangover and a random backoff — is sound; the recognisers ignore gain. It does not tell how to
   detect an unknown signal behind an AGC.

### 1.4 WALE (141D Appendix G)

141D **[full]**, including the image-only pages, rendered and read.

1. **Quantity.** Energy, against the quietest part of the band *now*: "An ALE channel or traffic
   sub-channel should be considered occupied if the energy in that (sub-)channel is at least 10 dB
   greater than that in the lowest-energy (sub-)channel of the entire wideband channel, which is
   used as the instantaneous noise floor for that wideband channel" (G.4.3.2).
2. **Noise reference.** The lowest-energy sub-channel of the same measurement: 1.5 kHz
   sub-channels across 24 kHz, or 3 kHz across 48 kHz. No memory, so no tracking delay.
3. **AGC.** Not discussed; a gain common to all sub-channels cancels in the comparison
   (inference).
4. **Decision.** tLBT and tLBR 400 ms by default (Table G-II). The occupancy recorded in the last
   scan cycle may stand in for a fresh listen (G.5.5.4.2).
5. **Performance.** "Detection probability of 99 % for each (sub-)channel having at least 10 dB
   more energy than the noise floor", tested with signals injected at RF. No false-alarm figure.
6. **Cost.** A band-energy comparison per sub-channel.
7. **Applies?** Yes, as a pattern: inside one SSB channel the analogue is the quietest bins of the
   same block. Its blind spot is a signal that fills the whole reference. G.4.1.6 adds an override
   mode because "spurious emissions and other interference could cause this adaptive system to
   avoid transmitting on channels that are actually usable".

### 1.5 STANAG 5066 and MIL-STD-188-110D

STANAG 5066 Ed. 4 public draft core text **[full]**, without its annexes (not public); Isode
white papers on Annexes K and L **[secondary]**; MIL-STD-188-110D **[full]**.

1. **Quantity.** Mainly decoded content. Each D_PDU carries the time to the end of its
   transmission, and timing from it "shall be used" when available (Annex K's slotted option,
   per Isode). Otherwise the end of the radio signal, with LBT_WAIT_TIME "current default is
   30 seconds", because "the radio signal may fade at the end". The draft core text says the
   standard "allows for, but does not require control communication with the modem". Annex L
   avoids collisions by token passing instead.
2. **Noise reference.** None: the inputs are decoded fields and the modem's carrier detect.
3. **AGC.** Not involved (inference).
4. **Decision.** Timers: a jitter or slotted backoff after the announced end.
5. **Performance.** Not published in what was read.
6. **Cost.** Protocol fields.
7. **Applies?** Aether already does the protocol part: its frames state how many follow
   (ADR-0068's counts) and stations reserve the channel after a heard session
   (`station/reserve.rs`). MIL-STD-188-110D's modem reports only its own waveform's carrier
   state (NO_CARRIER / CARRIER_DETECTED / RECEIVING) and has a "Half Duplex Receiver Master" mode
   that refuses to transmit until its receiver is back to NO_CARRIER: busy as acquisition, not
   energy.

### 1.6 Codan, Barrett and L3Harris radios

Codan 1997 reference manual **[full]**; Codan Envoy and Barrett 2050 manuals **[full, through
ManualsLib's partly garbled OCR]**; Barrett 4050 table of contents **[partial]**; no public
L3Harris operator manual was found.

1. **Quantity.** Syllabic voice squelch (speech cadence), Selcall and ALE decoding, optional data
   detection, and a signal-strength squelch. Codan Envoy's LBT listens for "Voice and calls" or
   "Voice, calls and data". Barrett 2050 halts a scan on "a signal with a level greater than the
   pre-set threshold" or when the syllabic mute opens.
2. **Noise reference.** The signal-strength squelch has a fixed low/medium/high threshold, not a
   learned floor. No dB values are given.
3. **AGC.** None of the manuals discusses it.
4. **Decision.** Codan Envoy LBT period 1–10 s (2 s by the OCR, uncertain), with "Override
   allowed"; scan dwells of 125–600 ms. Barrett resumes a scan once the signal has been below the
   threshold "for a period greater than the scan dwell period": the dwell is the hangover.
5. **Performance.** Not published.
6. **Cost.** Not stated.
7. **Applies?** The syllabic detector responds to envelope *variation* at speech rates, which a
   single AGC step is not (inference). The fixed S-meter squelch is the radio-side version of
   reading the S-meter over CAT (§4).

### 1.7 ITU-R SM.1880, SM.2256 and SM.1753

Rec. ITU-R SM.1880-3, Report SM.2256-2, Rec. SM.1753-2, SM.182-5 and F.1110-3 **[full]**.

1. **Quantity.** Occupancy, "the percentage of time that the signal is above a preset threshold
   level" (SM.1880).
2. **Noise reference.** SM.2256 §3.4 gives three ways, and is explicit that a once-measured noise
   level is "not advisable" below 30 MHz "because the noise level changes with time":
   - an unused frequency included in every scan, so the noise is "measured every revisit time
     just before the actual occupancy scan starts";
   - the channel's own free time slots (preferred);
   - the "80 % method" of SM.1753 §10.3: discard the highest 80 % of the samples and average the
     rest, recomputed "just before each single scan". Its stated weakness: "the calculated noise
     level rises when more channels are occupied with high levels … sensitivity is lost".
3. **AGC.** SM.1753-2 Table 4: "Measurement outputs should have no AGC applied."
4. **Decision.** A threshold at least 3–5 dB over the noise level, "otherwise short-term peaks in
   the noise level will result in phantom occupancy" (SM.2256). Integration over 5 or 15 minutes
   (SM.1880).
5. **Performance.** SM.1753 Attachment 2 validates the 20 % cut-off: any cut between 10 % and 70 %
   worked in its example.
6. **Cost.** A sort or a percentile over one scan.
7. **Applies?** Directly, as a principle: re-measure the reference from the same scan, from the
   quietest part of it. Aether's audio cannot be taken before the radio's AGC, so the same-scan
   reference is what cancels it.

**What the military and ITU sources share.** None specifies a long-memory learned noise floor.
Their detectors are waveform recognisers tested at fixed SNR on AWGN with no AGC in the loop, or
energy against a same-instant reference (WALE, SM.2256). Every system keeps an operator override,
because false busy is expected.

---

## 2. Amateur HF modems and programs

Source code was read at named commits; licences as found (ardopcf and ARSFI's HFSimulator are MIT;
fldigi, Direwolf, FreeDATA, Mercury and JS8Call are GPL; codec2 LGPL). The pattern first:

| System | What is detected | Noise reference | Learned floor? | Gain step | Hangover |
|---|---|---|---|---|---|
| ARDOP (ardopc, ardopcf) | spectral contrast in one FFT | other bins of the same FFT | no | cancels | 5 s |
| fldigi PSM / busy channel | max ÷ min of 10 Hz windows | weakest window, same frame | no (learns the ratio's distribution) | cancels | 3 s + 0–1 s |
| Winlink IONOS SIM detector | loudest 258 Hz window vs the rest | the rest of the band, same spectrum | per-bin averaging only | cancels | ≈ 0.3 s per bin |
| FreeDATA | bins over the frame's mean dB + 13 dB | mean of the same frame | no | cancels | up to ≈ 20 s |
| Direwolf | demodulator lock | none | no | immune | 6-of-32 symbols |
| FreeDV (codec2) | sync, SNR from the constellation | the constellation's scatter | no | immune | — |
| JS8Call | decodes in the last 30 s | — | — | immune | 30 s |
| Mercury (2026) | sync, preamble, NAV; optional energy | lower-quartile band, tracked | **yes** | **fails as Aether does** | 1.5 s |
| SCS PACTOR | FFT anomaly + PACTOR detectors | not published | not published | — | ≥ 3.5 s |
| VARA HF | not published | not published | not published | — | not published |

### 2.1 ARDOP

ardopcf (pflarue/ardop, `src/common/BusyDetect.c` `BusyDetect3`, `ARDOPC.c`
`UpdateBusyDetector`) **[source, MIT]**, compared with the four G8BPQ trees and the 2014–15 C#
and C versions **[source]**.

1. **Quantity.** Two contrasts from the sorted bins of one 1024-point FFT at 12 kHz (85 ms,
   11.7 Hz bins), over a span centred on 1500 Hz of half the ARQ bandwidth plus the tuning range
   each side: *narrow*, the mean of the 8 strongest bins over the mean of the rest; *wide*, the
   mean of the strongest two-thirds over the weakest third.
2. **Noise reference.** The rest of the same FFT. There is no learned floor.
3. **AGC.** A common gain multiplies every bin and cancels. A 2015 version also compared a fast
   and a slow average of the band's power ("start of a PACTOR frame or a static crash"); it was
   dropped, with the comment "May be issues with AGC speed".
4. **Decision.** Thresholds set by `BUSYDET` (0–10, default 5): narrow 3 + 0.008·B⁴, wide
   5 + 0.02·B⁴ (200/500 Hz modes). Busy after 3 busy calls in a row (≈ 0.3 s, every 100 ms); clear
   after 5 s since the last trip and 3 clear calls. Only evaluated while idle. `BUSYBLOCK` refuses
   a connect request unless the channel was clear ≥ 600 ms before it.
5. **Performance.** No published curves. A re-implementation of the statistic on idealised noise
   (our own script) found it gain-invariant (an AGC −20 dB step: 0 % busy), sensitive to
   narrowband signals, **blind to a signal that fills the span** (a 2.3 kHz signal never trips,
   even at +20 dB), and permanently busy when the rig filter is narrower than the span. As written,
   the running averages are local variables reset on every call, which makes the shipped
   statistic 0.2 × the instantaneous ratio, about 7 dB less sensitive than its thresholds suggest
   (from reading the code; not run).
6. **Cost.** One FFT and a sort of 206 values every 100 ms.
7. **Applies?** On the AGC axis, yes. Its price is the wide co-channel signal, which Aether's
   level path catches today and must keep catching.

### 2.2 WINMOR

Protocol specification 1.0.6 **[full]** (no busy detector in it), *A Winmor Primer* **[full]**,
the Winlink FAQ of 2019 **[full]**. The TNC was closed. Its indicator gave "the software busy
detector's best guess as to what kind of activity there is", narrow or wide, which matches the
2014 C# ARDOP code (inference). A FAQ contributor: a rig "BW much narrower than 1.4 KHz results
in Channel Busy ON all the time" — the signature of a spectral-contrast detector whose reference
extends past the rig filter. Noise reference, timing and performance: not published.

### 2.3 fldigi

`src/psm/psm.cxx`, `kiss_io.cxx`, `configuration.h` **[source, GPL]**.

1. **Quantity.** Across the modem's bandwidth plus 10 Hz, the waterfall's 1 Hz bins averaged into
   10 Hz windows; the statistic is the strongest window over the weakest. The source says the
   ratio is there "to deal with the AGC from radios". The ordinary squelch is a per-mode decoder
   metric (PSK: phase coherence) and gates text, not transmission.
2. **Noise reference.** The weakest window of the same frame. With the histogram option the
   threshold is *learned*: a histogram of the ratio, reset every 180 s, threshold at its fourth
   occupied bin above the lowest.
3. **AGC.** The ratio cancels it, and so does the learned threshold: what is learned is the
   distribution of a ratio, not a level. **This is the one adaptive scheme found that a gain step
   cannot move.**
4. **Decision.** A detection that persists 15 ms inhibits transmission for "Busy channel wait"
   (3 s default) plus 0–1 s random, then p-persistent CSMA (63/255, 100 ms slots).
5. **Performance.** Not published.
6. **Cost.** One pass over the bandwidth per waterfall update.
7. **Applies?** Yes. Its limit is ARDOP's: a signal covering the whole measured bandwidth leaves no
   quiet window.

### 2.4 Direwolf, FreeDV, FreeDATA, JS8Call

**[source]** for each.

- **Direwolf.** DCD is demodulator lock: per symbol, whether transitions fell where the DPLL
  expected; on at 30 of the last 32, off at 6 (AFSK 1200). No noise reference. Then CSMA (wait,
  DWAIT, p-persistence). Immune to AGC; sees only its own modulation.
- **FreeDV.** No busy detector. Sync from the unique word and pilots; SNR from the demodulated
  constellation (signal = mean |symbol|², noise = the scatter orthogonal to each symbol's axis).
  Immune to gain.
- **FreeDATA.** Per 100 ms, any bin above the *same frame's* mean dB spectrum + 13 dB marks its
  frequency slot busy; a leaky counter holds it up to about 20 s. Gain-invariant, but the mean
  includes the stopband above the rig's filter, so the threshold depends on the filter.
- **JS8Call.** Occupancy is decode history: an offset is taken if a JS8 decode arrived there in
  the last 30 s. Immune to gain, blind to anything that is not JS8.

### 2.5 Mercury (Rhizomatica)

`modem/channel_busy.[ch]`, `modem.c`, `datalink_arq/arq.c` at commit `182dda8` (2026-10-17)
**[source, GPL]**.

1. **Quantity.** Always on: decoder carrier sense (busy while any decoder held sync in the last
   250 ms; a caught preamble busy for its frame's airtime; a NAV header until the announced end).
   Optional, off by default: energy, the loudest 150 Hz band of 300–2700 Hz.
2. **Noise reference.** The lower-quartile 150 Hz band, tracked by an asymmetric average: toward a
   lower value at 0.20 an update (under a second), toward a higher one at 0.003 (≈ 17 s), and at
   0.0006 (≈ 80 s) while busy.
3. **AGC.** Not handled, and its own source comment states the consequence: "a step up in the
   noise itself … cannot be told from a wideband signal and reads BUSY until the floor catches up,
   ~100 s for a 12 dB step and ~145 s for 20 dB". **This is Aether's failure, in the one other
   modem that learns a floor the way Aether does.**
4. **Decision.** Busy at peak − floor ≥ 10 dB for 300 ms; clear below 7 dB for 1.5 s; not
   evaluated in a session.
5. **Performance.** As its comment says.
6. **Cost.** Small: bands of an FFT computed for display.
7. **Applies?** As a warning.

### 2.6 SCS PACTOR

DR-7800 manual on GitHub (`Commands.md`, `Firmware.md`) **[full]**; PTC-IIIusb manual
**[secondary, search extracts]**. In standby the modem calls the channel busy for signals
"audibly distinctly different from noise" slower than 250 Bd; "strong carriers on the channel are
not evaluated as channel busy". A universal FFT detector "sensitive for spectral peaks" looks for
spectral anomalies over 300–2700 Hz, alongside PACTOR-specific detectors (`CB`). Busy stays at
least 3.5 s. Internals and AGC advice: not published. Winlink's RMS Relay has an *Emphasize Pactor
signals* option because "the non-specialized busy channel process shows busy when it is not"
**[secondary]**.

### 2.7 VARA HF, Winlink, RMS Trimode and VarAC

Published documentation and observed behaviour only.

- **VARA HF.** Sends `BUSY ON` / `BUSY OFF` to the host (`docs/spec/host-interfaces.md`, from
  EA5HVK's *VARA Protocol Native TNC Commands*). The algorithm is not published. A changelog line
  seen only in a search extract **[secondary]**: v4.7.1 "Busy Detector moved after filtering stage.
  This avoid false busy flag by adjacent channels". Users report false busy cleared by turning
  off the rig's notch, NR and NB **[secondary]**.
- **Winlink.** The 2019 FAQ **[full]** puts listening on the calling operator ("simply listen on
  the frequency about to be used … If the frequency is occupied by any detectable signal … wait"),
  citing §97.221. KN6KB (2011) **[full]**: server-side busy blocking needs 2–3 s of quiet on a new
  frequency. Winlink's own experimental detector, the IONOS SIM BUSY DETECT (KN6KB, 2020, MIT)
  **[source and manual, full]**, compares the loudest 258 Hz window of a per-bin-averaged spectrum
  with the rest of the band, and its manual advises "The Receiver AGC should normally be set to
  FAST to MEDIUM".
- **RMS Trimode.** Consumes the modem's busy; its documentation could not be read (behind a bot
  challenge). This project's bench saw it toggle `LISTEN` every ≈ 3.5 s while scanning.
- **VarAC.** Consumes VARA's `BUSY`. "Beacons will not be fired if the freq is identified as BUSY …
  waits for 60 seconds from the moment the frequency is cleared before sending a beacon" (FAQ
  **[full]**). It has *Ignore VARA DCD* "to bypass constant noise on the frequency that prevent
  transmitting" (release notes **[full]**). On our bench it held "busy" about 10 s after each
  `BUSY ON`.

**What the amateur systems share.** Every detector that has lasted compares the channel with
*itself at the same instant* (a contrast inside one FFT) or with a demodulator's lock. Their price
is a signal that fills the reference region, and a rig filter narrower than the region. The one
that learns a level over time documents our failure. Field hangovers are 1.5–20 s; nothing found
holds busy for 70 s. Hosts already carry switches to ignore a false busy (VarAC's *Ignore DCD*,
Pat's `--ignore-busy`, ARDOP's `BUSYDET 0`), so an over-sensitive detector gets turned off.

---

## 3. The detection and noise-floor literature

### 3.1 Energy detection and the SNR wall

Urkowitz, "Energy detection of unknown deterministic signals", *Proc. IEEE* 55(4), 1967
**[abstract]**; Tandra & Sahai, "SNR walls for signal detection", *IEEE J-STSP* 2(1), 2008
**[secondary]**, its result read **[full]** in Tandra's thesis (UC Berkeley, UCB/EECS-2009-192),
Theorem 2.1.

1. **Quantity.** Energy in a band over N samples.
2. **Noise reference.** Must be known or estimated separately.
3. **Gain.** Not invariant. With the noise known only within a factor ρ (x = 10 log ρ dB), the
   radiometer's wall is SNR_wall = (ρ² − 1)/ρ: below it no N and no threshold meet both targets.
   For x = 3, 10, 20 dB the wall is +1.7, +10, +20 dB (derived). An AGC hold-down of 10–20 dB is
   exactly such an uncertainty.
4. **Decision.** A threshold.
5. **Performance.** Sample complexity grows without bound at the wall.
6. **Cost.** The cheapest detector.
7. **Applies?** It is Aether's level path. The way out the thesis gives is **run-time noise
   calibration** (§3.2.2): measure a second band with an identical filter at the same time and
   test the difference. That removes the level uncertainty and leaves only the uncertainty in the
   noise's *colour* between the two bands — here the radio's filter shape, which is fixed and
   learnable. It needs a band the signal leaves free.

### 3.2 Noise-floor trackers, and stagnation

Martin, "Noise power spectral density estimation based on optimal smoothing and minimum
statistics", *IEEE T-SAP* 9(5), 2001 **[partial]**; Cohen & Berdugo, *Signal Processing* 81, 2001
**[full]**; Cohen, IMCRA, *IEEE T-SAP* 11(5), 2003 **[partial]**; Cohen & Gannot, *Springer Handbook
of Speech Processing* ch. 44 **[full]**; Stahl, Fischer & Bippus, quantile noise estimation,
ICASSP 2000 **[secondary]**, described in Bonde et al., NOLISP 2005 **[full]**; Gerkmann &
Hendriks, "Unbiased MMSE-based noise power estimation…", *IEEE TASLP* 20(4), 2012 **[full]**.

1. **Quantity.** A per-bin noise level from the noisy power over time.
2. **Tracking.** Minimum statistics follows a fall at once and a rise within its window (1–3 s in
   speech use). MCRA and IMCRA recover within under a window: IMCRA's speech-presence gate is
   referenced to a *first, ungated* minimum, so the gate cannot lock against a stale estimate of
   its own. Quantile estimation follows within a fraction of its window and has no gate at all.
3. **Gain.** None is invariant: all track absolute level, so all learn a hold-down. They differ
   only in how fast they climb back.
4. **Decision.** Not a detector; a floor.
5. **Performance.** Gerkmann & Hendriks name our deadlock **stagnation**: when the estimate is far
   below the true noise, the speech-presence probability goes to one and "the noise power will not
   be updated anymore". Their cure forces an update when the smoothed probability stays above
   0.99; an earlier estimator used a safety net, never below the minimum of the last 0.8 s.
6. **Cost.** A few operations per bin per frame.
7. **Applies?** Two lessons transfer. Aether's evidence gate ("not over the floor + 6 dB") is a
   hard speech-presence gate pointed at its own estimate: it is what turns a median's bounded
   delay into 70 s. And there is a limit: speech has pauses within seconds, HF does not (FT8
   periods of 12.6 s, RTTY for minutes, ARQ exchanges). **A floor that rises fast enough to follow
   an AGC release also learns a long steady signal. Inside the channel the two look the same.**
   Only a cue from outside the channel separates them.

### 3.3 FCME, LAD and CFAR across frequency

Vartiainen, *Concentrated signal extraction using consecutive mean excision algorithms*, Univ.
Oulu 2010 **[full]**, summarising Saarnisaari (PLANS 2004) and Vartiainen, Lehtomäki, Saarnisaari &
Juntti (J. Electrical and Computer Eng. 2010, **[abstract]**); US patent 7,231,194 (LAD)
**[full]**; Rohling, "Radar CFAR thresholding in clutter and multiple target situations", *IEEE
T-AES* 19(4), 1983 **[full]**.

1. **Quantity.** Outlier bins in one block's spectrum.
2. **Noise reference.** FCME sorts the bins, takes the smallest 10 % as a clean set, sets a
   threshold at T·mean of the clean set, adds every bin below it, and repeats. Ordered-statistic
   CFAR takes the k-th smallest of N reference cells (Rohling: N = 24–32, k between N/2 and 3N/4).
3. **Gain.** Exactly invariant: the threshold is a multiple of the same block's quiet bins.
4. **Decision.** T = −ln(P_FA) for power bins: 4.6 for 1 %, 6.9 for 0.1 %. LAD runs two factors
   and keeps clusters that cross the upper one.
5. **Performance.** FCME's threshold held with signals in up to 80 % of the bins, still detected
   at 85 %, failed at 95 % "because the initial set was not clean". OS-CFAR tolerates interferers
   among its reference cells.
6. **Cost.** A sort of a block's bins: negligible at 25 ms.
7. **Applies?** Yes, over the **whole audio passband**: the quiet bins outside Aether's channel are
   the reference, in the same block. The radio's filter shape makes the noise non-white; a slowly
   learned per-bin noise shape (a gain-free ratio) corrects it. A 2300 Hz channel inside a 2.4 kHz
   SSB filter leaves little passband outside it.

### 3.4 Eigenvalue and covariance detectors

Zeng & Liang, eigenvalue-based sensing, *IEEE T-COM* 57(6), 2009 (arXiv:0804.2960) **[full]**, and
covariance-based sensing (arXiv:0808.2562) **[full]**; Bollig et al., arXiv:1610.03892
**[abstract]**. The maximum-to-minimum eigenvalue ratio, or the covariance absolute value, of
consecutive samples: a ratio, so gain-invariant, with a threshold from random-matrix theory
independent of noise power. But the noise must be white (Aether's channel at 8 kHz is heavily
band-filtered), uncertain colouring brings its own SNR wall (Bollig), the papers use 10⁴–10⁵
samples where a 25 ms block has 200, and a channel-filling OFDM signal looks nearly white. Cost
about ML (eigenvalue) or L (covariance) times an energy detector. **Not a floor estimator.**

### 3.5 Cyclostationary detection

Gardner, "Signal interception…", *IEEE T-COM* 36(8), 1988 **[abstract]**; Tandra thesis ch. 3
**[full]**; Urriza, Rebeiz & Cabric, arXiv:1211.0313 **[full intro]**. Spectral correlation at
known cycle frequencies; stationary noise has none, and normalised forms are gain-invariant. It
needs the cycle frequencies (symbol rate, prefix) known, and fading imposes its own SNR wall.
Aether already uses it for its own signal (preamble and prefix correlation). For unknown HF users
at unknown rates and offsets it is neither general nor cheap.

### 3.6 Spectral flatness

Definition via Johnston, *IEEE JSAC* 6(2), 1988 **[secondary]**; Moattar & Homayounpour, "A simple
but efficient real-time voice activity detection algorithm", EUSIPCO 2009 **[full]**. The
geometric over the arithmetic mean of the power spectrum: gain-invariant; plain noise reads about
−2.5 dB on a raw periodogram (derived). It misses a noise-like signal that fills the channel.
Moattar & Homayounpour show the useful structure: the energy floor is updated only on frames judged
silent, but the vote has two gain-invariant features, so silence can still be declared after a
level step and the floor keeps updating. Aether's shape path (peak ≥ 12 dB over the median bin)
is already a flatness-type cue, and it is unaffected by the AGC.

### 3.7 HF occupancy surveys

Pantjiaros, Laycock, Gott & Chan, *IEE Proc. Commun.* 144(1), 1997 **[abstract]**; Pantjiaros et
al., *IEE Proc. Commun.* 145(3), 1998 **[abstract]**; Stehle & Hagn, *Radio Science* 26(4), 1991
**[abstract]**; Haralambous & Mostafa, IES 2015 **[full]**; ITU-R WP5A working documents 5A/79 and
5A/198, Annex 12, 2012 (CRC measurements) **[full]**. Measurement receivers with calibrated, fixed
gain: absolute field-strength thresholds chosen so that a noise sample rarely exceeds them, or
(CRC) a noise floor estimated over a **180 kHz** sliding window and a channel occupied 10 dB above
it. Administrations objected that 10 dB "may find far less activity" than amateurs hear. No AGC by
design. The lesson for Aether: a reference much wider than the channel, so that one signal cannot
dominate it.

### 3.8 AGC theory and the radios

FTDX10 operation and CAT manuals **[full]**; IC-7300 full manual **[full]**; AB4OJ on Icom's AGC
**[full, an expert's note]**; Elecraft K3S and K4 manuals **[full]**; a FlexRadio application note
**[full]**; CuteSDR source **[source]**; W3PFR on Elecraft hardware AGC **[secondary]**.

- **Attack, hold, decay.** Attack in milliseconds; a hold after the signal falls below the AGC
  detection level; then the decay. The FTDX10 manual: the decay applies "after the input signal
  level becomes lower than the AGC detection level and the HOLD time has expired".
- **FTDX10.** AUTO selects SLOW in SSB, **MID in DATA-L/U, PSK and RTTY**, FAST in CW and FM. Decay
  defaults: SSB 300/1000/3000 ms; DATA, RTTY, CW 160/500/1500 ms; AM 1000/2000/4000 ms; each
  20–4000 ms. Roofing filters 500 Hz, 3 kHz and 12 kHz at the first IF, switched with the mode. The
  manual does not say where the AGC detector sits. Aether's own measurement on this rig (beta.17,
  AUTO): a step down in a millisecond, about 100 ms of hold, 40–55 dB/s back, triggered from
  outside the audio passband.
- **IC-7300.** SSB 0.3/2.0/6.0 s; CW and RTTY 0.1/0.5/1.2 s. Its manual: "When you are receiving
  weak signals, and a strong signal is momentarily received, the AGC function quickly reduces the
  receiver gain."
- **Before or after the narrow filter.** A DSP AGC after the IF filter sees only its passband
  (CuteSDR; the IC-756Pro3: "Signals outside the DSP IF passband do not develop AGC voltage").
  A primary loop ahead of the narrow filter sees everything inside the roofing filter (IC-7700/7800
  per AB4OJ; Elecraft's hardware AGC: "activated by the strongest signal within the passband of
  the roofing filter"; the K4's automatic front-end attenuator). Either way, **the whole audio
  passband is scaled by the same factor**, which is what makes a same-instant reference work.
- **The 70 s is ours.** The radios' decays are a few seconds at most.

### 3.9 AGC-aware detection

van Bloem, Schiphorst & Slump, *EURASIP J. Adv. Signal Process.* 2013:72 **[abstract]**: in
spectrum measurements the AGC "raises the noise level when strong adjacent signals are present",
and since the AGC is common to all frequencies, a factorisation of the time-frequency data removes
it (offline, costly). Akos, *NAVIGATION* 59(4), 2012 **[abstract]**, and Axell et al.,
*NAVIGATION* 62(1), 2015 **[abstract + intro]**: GNSS receivers use their own AGC level as an
interference detector. Its HF analogue is reading the radio's S-meter over CAT as a gain witness
(§4).

---

## 4. What the radio itself can tell

Every command was read in the maker's manual: FTDX10, FTDX101MP/D and FT-991A CAT references
**[full]**; FT-710 CAT reference (Japanese, rev 2212-B; the ASCII syntax read) **[full]**; Kenwood
TS-590S/SG and TS-890S PC command references **[full]**; IC-7300 full manual §19, IC-705 and IC-7610
CI-V references **[full]**; Hamlib master `da925ab` and FLRig 2.0.12 **[source]**.

| Radio | Read | Answer | Meaning |
|---|---|---|---|
| FTDX10, FTDX101, FT-991A, FT-710 | `SM0;` | `SM0nnn;` | S-meter 000–255, raw counts; never pushed, so polled |
| FTDX10, FTDX101, FT-991A | `BY;` | `BYab;` | "RX BUSY ON (SQL OPEN)": squelch open, not a carrier detect |
| FT-710 | `RI0;` | `RI0abcdefg;` | no `BY`; field P8 is the squelch |
| FTDX10 (and the others) | `GT0;` | `GT0n;` | AGC 0 off, 1–3 fast/mid/slow, 4–6 *which one AUTO chose* |
| TS-590S/SG | `SM0;` | `SM0nnnn;` | 0–30 meter dots (about 4 dB a dot) |
| TS-890S | `SM;` | `SMnnnn;` | 0–70 dots, no parameter |
| TS-590, TS-890 | `BY;`, `GC;` | | busy LED; AGC off/slow/(mid)/fast |
| IC-7300, IC-705, IC-7610 | `15 02` | `15 02 hh ll` | BCD 0000–0255; 0120 = S9, 0241 = S9 + 60 dB |
| IC-7300, IC-705, IC-7610 | `15 01`, `15 07` | | squelch status; the OVF (ADC overflow) flag |
| IC-7300, IC-705, IC-7610 | `16 12`, `1A 04` | | AGC fast/mid/slow; time constant (00 = off) |
| Hamlib | `l STRENGTH`, `\get_dcd` | | dB relative to S9 from rough tables (the FT-991 and FT-710 ones marked "TBC"; the FTDX10 a generic Yaesu table); DCD is "squelch open", and Yaesu's backends return "not available" |
| FLRig | `rig.get_smeter` | | 0–100, 50 = S9; FLRig's own cached reading, up to 500 ms old by default; no squelch or busy call |

1. **Quantity.** The S-meter follows the AGC: above the AGC threshold the audio stays level while
   the meter rises about dB for dB; below it the meter sits near its floor (Yaesu, Icom and Kenwood
   manuals; the TS-590SG manual says the meter reacts with the AGC's time constant).
2. **Reference.** Its calibration is coarse: Icom S-units are about 2–3 dB below S9 (TAPR DCC 2018
   **[full]**), and HamSCI found S9 anchors off by ±7 dB **[full]**. It is a usable *relative* gain
   witness above the threshold, not an absolute one.
3. **AGC.** It is the AGC's own state, but **whether the FTDX10's `SM0;` follows the gain cuts its
   recordings showed is not documented**: if the S-meter reads only the DSP AGC and the cut is
   made by a loop ahead of the narrow filter, the meter will not show it. The test is to poll it
   at 20 Hz while recording an idle band (the S-meter tool proposed in `00-recording-plan.md`).
4. **Decision.** A gate ("the AGC is acting: do not learn the floor") rather than a calibrated
   correction; or a per-station fit of the meter's dB map against the audio's drops.
5. **Squelch as busy.** Not usable as is: with the squelch open, as it normally is on SSB and DATA,
   `BY`, Icom `15 01` and Hamlib's DCD all read busy all the time.
6. **Cost.** A poll is about 3 ms on the wire at 38 400 Bd, but about 20 ms a round trip on Yaesu
   and 45–65 ms on CI-V at 9600 Bd (ADR-0071's measurements of keying commands). 5–10 polls a
   second while idle, none while keyed, at the lowest priority on the CAT thread.
7. **Applies?** Only on radios with CAT, through Aether's own CAT or a server; not with a serial
   line, VOX or a host program keying. A fallback is always needed.

---

## 5. Operating practice

| Source | Advice | Why |
|---|---|---|
| WSJT-X user guide **[full]** | "usually best to turn AGC off or reduce the RF gain control to minimize AGC action" | many signals share one passband |
| ardopcf user guides **[full]** | turn the AGC off, ride the RF gain by hand; turn off Windows' input enhancements | "AGC attempts to keep the average power level across the total receive bandwidth relatively steady" |
| fldigi help **[full]** | transmit no more than needed | a strong signal "captures your AGC" |
| Winlink IONOS SIM manual **[full]** | AGC FAST to MEDIUM | for its busy detector |
| IC-7300 manual **[full]** | after a strong burst, FAST or OFF | the weak signal is lost "because of the AGC action" |
| FTDX10 manual **[full]** | leave AUTO (MID in DATA) | not stated |
| TAPR DCC 2018, IC-7300 at 630 m **[full]** | AGC on costs little: an interferer at S9 + 20 in the passband did not hurt −24 dB WSPR decodes | measured |
| K0PIR guides **[secondary]** | fast AGC for ARDOP; "Use 'Slow AGC'" for VARA | not stated |
| SCS P4dragon manual **[full]** | no AGC advice; no external DSP; notch and NB off | latency |
| MIL-STD-188-141 **[full]** | data-mode AGC: attack ≤ 10 ms, release ≤ 25 ms | — |

The advice disagrees: off, fast, medium and slow. The measured counterpoint (TAPR) explains why a
decoder does not care: the AGC leaves the SNR alone and moves the *absolute* audio level — exactly
what an absolute-level floor misreads. A station running Aether in USB rather than DATA-U on an
FTDX10 gets AUTO = SLOW, 3 s of decay, rather than MID's 500 ms.

---

## 6. Ranking for Aether's failure

**Ideas that address an AGC held down by a signal outside the detector's channel**, best first:

1. **A same-instant reference from the radio's audio outside the channel** — run-time noise
   calibration (Tandra), WALE's "lowest-energy sub-channel … as the instantaneous noise floor",
   SM.2256's per-scan reference, ARDOP's and fldigi's contrasts, made robust by an order statistic
   (OS-CFAR, FCME, SM.1753's 20 % method). The AGC scales the whole passband by one factor, and a
   signal can only *add* power, so a low quantile of the passband's spectrum can fall only when the
   gain or the band's noise really fell. Cheap, needs no radio help, works whatever keys the radio.
   Limits: a signal that fills the reference (the in-channel energy must still be judged against a
   floor that the reference cannot raise), a rig filter narrower than the reference band, and a
   2300 Hz channel that leaves little passband outside it.
2. **A gain-event or stagnation guard on the floor** (Gerkmann & Hendriks; IMCRA's ungated
   reference): bounds how long a stale floor can survive. In-channel only, so on its own it cannot
   tell an AGC release from a long flat signal starting.
3. **The radio's S-meter over CAT** as a gain witness or a "do not learn now" gate: direct, but
   coarse, radio-specific, CAT-only, and on the FTDX10 not yet known to show the cuts at all.
4. **Operator guidance** (AGC FAST or DATA mode; the panel saying when an AGC acts): helps, removes
   nothing.

**Ideas that work only inside the channel** (they survive a gain step but cannot see why the floor
moved): spectral flatness and the existing shape path (narrowband signals); decoder and preamble
carrier sense (Aether's own frames, already used); eigenvalue and cyclostationary detectors (known
or correlated signals); in-channel contrasts like ARDOP's span (blind to a channel-filling signal).

**Ideas that need the radio's help**: the S-meter or AGC level over CAT; the AGC setting (`GT0;`,
`16 12`, `GC;`) to tell the operator which is in force; the radio's squelch only if the operator
sets one.

**Ideas that fail under a whole-passband gain step**: anything comparing the channel's absolute
level with its own history — energy detection against a learned floor, minimum statistics,
quantile and MCRA trackers with windows long enough to ignore long HF signals, Mercury's floor, and
Aether's detector today.
