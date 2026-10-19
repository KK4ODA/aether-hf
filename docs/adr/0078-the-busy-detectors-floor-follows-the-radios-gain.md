# ADR-0078: The busy detector's floor follows the radio's gain

**Status:** accepted, 2026-10-19, at the end of the investigation the author asked for after
beta.113's panel showed an idle 3.590 MHz channel busy by level for about seventy seconds, its
floor slid to −47 dBFS (`docs/research/busy-detection/`: the recording plan, the survey, the
measurements, the candidates and the criteria registered before they were run). The busy detector
lives in the daemon alone (`aetherd/src/busy.rs`); there is no model of it to change first. **No
wire change, no configuration key, no schema bump**: what a station sends is untouched, and a
station on this build works with any protocol-9 station. The author's recordings
(`00-recording-plan.md`) are owed: everything here was measured on synthetic audio and the
harness, with the AGC model beta.17 measured.

## 1. Context

A receiver's AGC turns the whole audio passband down the moment a strong signal appears anywhere
in it — in the channel, beside it, or outside the DSP filter altogether — holds it down while the
signal lasts, and lets it back when it goes. The busy detector (ADR-0010's floor, beta.17's steady
blocks, ADR-0076's frame spans) learned its noise floor as the median of the last ten seconds'
quiet, steady blocks, in dBFS. Under the cut the channel's noise is quiet, steady and low: every
test of evidence passed, and the floor followed it down. When the AGC let go, the true noise stood
over the floor by the cut; no block was evidence any more (each was over the threshold), the floor
was held for its 60 s, and the channel read busy until the hold ran out and the floor took the
median of everything — 70–73 s, the window plus the hold plus the hangover. The literature calls
it *stagnation*: the evidence gate referenced the floor it updated (`01-survey.md` §3.2).

How often: KK4ODA-1's daemon logs, 108 hours from 2026-10-08 to 10-17, hold 57 such spans (idle,
busy by level for 20 s or more, ending with the floor at least 6 dB higher than it began) — 0.53
an hour, 60 minutes busy in all, 36 of the 57 between 69 and 73 s, two-thirds of them on 80 m
between 00 and 04 UTC (`02-measurements.md` §1). Each held back every call, probe, beacon and
datagram the station would have started.

The survey ranked first a reference taken at the same instant from the radio's audio outside the
channel, which the AGC scales by the same factor: noise-calibrated spectrum monitors and cognitive
radios do this (run-time noise calibration, SM.2256's per-scan reference, WALE's quietest
sub-channel), and Mercury's documentation describes the same failure without a fix. The S-meter
over CAT is coarse (2–4 dB a step under S9), radio-specific and absent on a serial-line, VOX or
host-keyed station; a radio's busy or squelch status reads an open squelch. Neither can be the
fix.

## 2. Decision

1. **The station hands the detector the radio's whole audio** as well as the channel's baseband
   (`BusyDetector::hear_passband`, called by `Station::capture` before `push`, skipped while the
   station is deaf). A `PassbandReference` reads the radio's gain from it every 25 ms:
   * a 1 200-point four-term Blackman–Harris transform of the 48 kHz audio (40 Hz bins; a carrier
     40 dB over the noise lifts ±4 bins through it, against ±10 through a Hann window);
   * the bins of 300–2 700 Hz, leaving out the channel as its band filter hears it (the occupied
     band ± 280 Hz) when 20 or more remain — at 500 Hz 34 of them; a 2 300 Hz channel fills the
     band, and there every bin is read (`includes_channel`);
   * each bin's level, averaged over four blocks, against **its own level on quiet**: set at first
     to the bin's 25th percentile over a two-second warm-up (`warm_up`; a static crash in it moves
     no quantile), then a quantile tracker at 25 % (down 0.02 dB a reading when under, up a third
     of that when over) that learns only from a reading that is uncut (≥ −3 dB) at a block the
     channel's own tests took as evidence of the noise (`learn`);
   * the gain is the 25th percentile of the bins' differences from their levels on quiet — a cut
     moves every bin down together, a signal only lifts the bins it holds, so up to three bins in
     four may hold one and the reading stays on the noise; the same quantile on both sides, so
     noise alone reads 0 dB with no correction;
   * only bins whose level on quiet is within 10 dB of the channel's count (a bin outside a narrow
     radio filter holds the sound card's own noise; a bin far above learned a signal that was there
     more often than not), and only when ten or more remain;
   * the reading is the median of the last three (75 ms): one reading wanders ±2 dB on noise, and
     more of them put the reading behind a fast AGC. With no reading the floor holds where it was
     and the bins learn nothing.
2. **The floor is learned against the gain and drawn at the gain of the moment.** The floor is the
   median of `power / gain` over the window's evidence, and is drawn at `floor × gain` for every
   block. The gain it is drawn at (`draw`) is the reading, never above 0 dB — a signal only adds
   power, so a reading over the levels on quiet is a signal, not more gain — taken at once when it
   moves more than 2 dB and eased toward with a second's time constant when it moves less (the
   reading wanders about a decibel on noise, and a floor drawn at every wander judged a signal at
   the threshold against a moving line). A cut lowers the floor at once and the gain coming back
   raises it at once; the evidence gate never stagnates, because the noise under a cut and after
   it are the same number in the floor's units.
3. **Where the reading includes the channel (2 300 Hz), the floor follows it down at once but back
   up only once it is within 3 dB of 0.** There a wide signal in the channel lifts the reading
   exactly as the gain coming back would, and a station's own channel is what the detector must not
   miss; the price is a slow AGC's ramp read as busy until it is nearly done (§3).
4. **When the hold runs out, the gain of the moment becomes the normal one** (`rebase`): the
   levels on quiet move by it and the floor's units with it, so the floor in dBFS is unchanged. A
   reading stuck under 0 dB — levels on quiet learned before an attenuator went in, say — would
   otherwise keep a 2 300 Hz floor from following the gain back up after every cut.
5. **The operator is told.** While the gain reads 6 dB or more down, the Diagnostics tab's busy
   line says *radio gain −20 dB* (the metrics' `radio_gain_db`), and a busy transition logged then
   ends with `| radio gain −20 dB`.
6. **Tools**, kept: `aetherd --busy-trace WAV [--expect SIDECAR] [--bandwidth N] [--busy-variant
   plain]` (the detector over a recording, block by block, as CSV — `plain` is the floor in dBFS,
   as before, kept to replay a recording both ways), `tools/floor_trace.py` (every AGC event in a
   recording: depth, length, release ramp, where its cause sat, what the floor did, how long the
   channel read busy after it), `tools/busy_corpus.py` and `tools/busy_eval.py` (the corpus and the
   judge of `03-candidates.md`), the channel server's `carrier` QRM and `windows` keying, and the
   harness's steps `at`, `clear` and `busy`.

Not built (`03-candidates.md` §1, §3): a floor memory from the channel alone (C1: releases of
2–8 s on 24 clips), refusing evidence while the gain is cut (C3:
misses signals in the channel under a cut — 19 clips), a shorter hold (C6: still 27 s), a
stagnation guard (C7: C6 by another name), and the S-meter (C5: a witness at most, later).

## 3. Measured

**The corpus** (`tools/busy_corpus.py`, 183 clips, 6.7 hours of 48 kHz audio: band noise through a
2.7 kHz filter, the harness's signals keyed on fixed windows, the harness's AGC on four presets
read from the FTDX10's manual and beta.17's measurement; `03-candidates.md` §2–3). The final build,
before → after:

| Group | Before | After |
|---|---|---|
| A — a strong signal outside the 500 Hz channel, 96 events, every AGC preset | every event busy until the next or the clip's end (40 s); 3 839 s of false busy | clear within 1.2 s of every event; 1 s |
| H — a signal the AGC feels and the audio never carries, 36 events | 40 s, every one; 1 436 s | 500 Hz: never busy; 2 300 Hz: up to 5.8 s; 41 s |
| L — a two-minute cut | 70 s, every one | 500 Hz: never busy; 2 300 Hz: up to 3.0 s |
| C — a signal in the channel under a cut | 350 s of false busy after the cut | none; the signals caught as before (0.93 of them) |
| F — a strong signal in the channel driving the AGC | 57 s | 20 s, all with a radio filter too narrow to read the gain |
| B, D, E, G — no AGC trouble | — | as before, within the floor's own noise |

**Held out.** Run on a second draw of the same conditions (`--seed 1`), the build before last
found a defect the first draw had not — each bin's level on quiet started from one reading, and a
static crash in it made every later reading a 9 dB cut (busy 47 % of the time with no AGC); the
warm-up now takes a quantile (§2.1). The final build was then run once on a third draw
(`--seed 2`) that nothing had been looked at on: group A clear within 1.2 s of all 96 events
(before: 95 never, 40 s), H and L as above (2 300 Hz up to 6.1 s), C's false busy 350 s → 0, F's
67 → 19 s, B, D, E and G as before. Nothing new failed.

**The session recordings.** KK4ODA-1's 17 recordings of 2026-10-10…17 replayed both ways
(`02-measurements.md` §8). They show the premise on the author's radio: when the other station
transmits, its own signal drives the AGC, the passband outside the channel falls 17–25 dB, and the
gain reading follows. Where nothing should be on the air (798 s) the new floor reads busy by level
33 s more than the old, and those blocks have the channel a median 7.6 dB above the rest of the
passband — energy the old floor had taken in (it sat at −18.4 dBFS over a −27 noise on one), not
false alarms.

**The harness** (`02-measurements.md` §7): ten scenarios, three draws, against beta.114's daemon.
With a carrier 40 dB over the noise outside a 500 Hz channel for 30 s (AGC AUTO, and SLOW),
beta.114 fails all six runs — the channel never reads clear within 6–8 s of the carrier going, and
no call is made — and this build passes all six. Every scenario passes at least as often (30/30
against 24/30), no radiated overlap in either arm, nothing slower by the nightly's trend rules.

**Cost:** 10.7 µs more a 25 ms block (21.0 against 10.3 for the detector alone, release build,
2.1 GHz Xeon): 0.04 % of a core.

**What the registered criteria still fail**, each traced (`03-candidates.md` §3.2–3.3):

* weak signals right on the threshold (0 dB in 3 kHz, 6.0–6.5 dB over the channel's noise), most
  under a cut, called busy 0.05–0.6 s later than by the old floor on five clips of 183 (two on the
  held-out draw); the two floors differ there by a median −0.17…+0.12 dB with the sign varying, and
  over every transmission of the three draws the new floor called the first busy earlier 34 times,
  later 11, at the same block 254 — chance on a threshold, not a bias;
* one busy share 0.006 under the old floor's with no AGC (0.005 allowed);
* the floor's 95th-percentile error over 3 dB where the clip's own truth is not the channel's: a
  +40 dB RTTY's keying sidebands put noise-level energy into the channel 1 kHz away (the old floor
  misses by 26–31 dB there, the new by 5–6), static crashes move a fast AGC faster than the 1 s
  median the truth is drawn from, and a dense band of strong FT8 beside the channel hides the gain
  (the new floor there reads exactly as the old);
* at 2 300 Hz with a slow AGC and a deep cut from a signal the audio never carries, the channel
  reads clear 3.3–6.1 s after the gain starts back (§2.3), against the 3 s registered for that
  group's kind (it was added after the first run).

## 4. Consequences

* The channel no longer reads busy for a minute after every strong signal near it; at 500 Hz it
  reads clear within about a second of the AGC letting go, at 2 300 Hz within the AGC's own ramp.
* A radio filter narrower than about 1 kHz around a 500 Hz channel leaves no bins outside the
  channel carrying the band's noise: there is no reading and the floor behaves as before. With a
  slow AGC and a strong signal in the channel, that is the 20 s left in group F. A dense band of
  strong signals filling the rest of the passband hides the gain the same way.
* In a session the other station's own signal drives the AGC, and the floor follows the cut: the
  panel's floor dips with each of its transmissions, and the channel reads busy while it sends —
  as it should; the old floor had risen into the other station's signal.
* The busy detector now does an FFT of the radio's audio every 25 ms: 0.04 % of a core.
* The field guide no longer needs to ask for AGC FAST or AUTO; it still prefers DATA-U to USB on
  an FTDX10 (AUTO is SLOW in USB).
* Open: the author's recordings must show that a real radio's passband outside the channel follows
  its gain as the model's does, fit the presets, and say where the signals that cut the gain sat
  (`02-measurements.md` §6); the same recordings replayed through `tools/floor_trace.py` are the
  field check of §2.4 of the criteria.
