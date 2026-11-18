# ADR-0132: The receive filter is not read; the waterfall shows it

**Status:** accepted, 2026-11-18: the author's decision after `docs/research/receive-filter/06-r4-results.md`
and `08-r5-results.md`. **Supersedes ADR-0105**, except its §2.2 (clipping), which stays. **No wire
change, no configuration key, no schema bump.** The control API's `metrics` loses `rx_filter`, and a
recording loses its `passband` section.

## 1. Context

ADR-0105 (beta.145) gave the station a reading of the radio's receive filter, made from the band's
noise while the station listens, with a banner on every tab when an edge of the signal was cut. It
replaced an estimate that had failed three times.

On 2026-11-17 the reading warned at both stations on 7.101 MHz, with both radios' filters set right
(`05-busy-band.md`). Other stations' signals, wider than the reading leaves out, look like a filter.
Two more readings were then built and judged against criteria fixed before any run:
* R4 (`06-r4-results.md`) gave fewer false warnings after the first half minute, but missed the
  criteria;
* R5 (`08-r5-results.md`) missed more of them.

Some cases no reading of this kind can clear. It tells a signal from the filter by watching the
signal come and go, so it fails:
* in the station's first half minute, when another station was already on;
* on a signal that is on nearly all the time;
* at 500 Hz, on another signal about as wide as Aether's own.

A false warning tells an operator to turn a knob that is already right. New operators would meet it
on any busy band. No other modem reads the filter: WSJT-X, fldigi, ARDOP, FreeDATA, JS8, Mercury and
VARA leave it to the operator (`02-survey.md`).

The author (2026-11-18): *"I would remove this filter warning all together, false warnings will
generate a significant amount of noise when new users start using it. No other modem incorporates
it. Unless we can find a super simple and robust solution to this problem, like looking at the
waterfall does."*

## 2. Decision

**The station does not read the receive filter.** The operator checks it on the waterfall. Its red
lines are the edges of Aether's signal. The band's noise should look even between them, and noise
that fades out inside a red line is the filter cutting that edge. The manual says so where the red
lines are explained (§11) and in *The receive audio* (§28, Figure 33), with the knob to turn for
each edge. The waterfall needs nothing new: it has drawn the red lines since the redesign.

**Clipping stays** (ADR-0105 §2.2). The share of the audio at full scale is a count of samples. It
cannot mistake another station for clipping, and its knob is the same on every radio. It moves to a
module of its own (`crate::clipping`). Its banner shows on every tab, Compact included, and the
Receive level line reads it.

**ADR-0104 stays.** The receivers read the noise across the band, so a cut costs less than it did:
on the mobile's cut of 2026-11-07, OFDM rungs 13 and 16 went from 0 to 30 decoded of 30.

## 3. What it removes

* The daemon's reading: `crate::passband` (`ReceiveFilter`, `FilterReading`, `Verdict`, the
  receiver test noise) and its hooks in the station (each look, each frame's span, the bandwidth
  move, the scan's hops).
* `aetherd --passband-trace` (`passband_trace.rs`).
* `metrics.rx_filter` and a recording's `passband` section (its noise profile and its readings).
* The Log's `filter` line (L1, which went out in no release).
* The spectrum analyser's second job: `compute_to` and `window_s`. It is the display's again.
* The panel's receive filter banner, its Dismiss and its Setup step 4 button, the Status tab's
  *Receive filter* line, and their styles.
* `tools/rxfilter_corpus.py`, the harness's `filter_warnings` column (H1), and the nightly's list
  of them.
* Their tests.

Nothing is added. The manual's screenshots of the Status tab and the banners are taken again. The
banners picture shows the clipping banner in place of the filter's.

The research stays in `docs/research/receive-filter/`. Its scripts and `rxfilter_corpus.py` run at
`b31eb8d`, the last tree with the reading. Kept because they measure the receivers, not a warning:
* the channel server's radio (`rx_filter_hz`, `rx_floor_db`, `rx_gain_db`);
* `tools/rxfilter_cost.py`, `tools/bench_cut.py` and `tools/passband_witness.py`.

## 4. Consequences

* An operator whose filter cuts the signal is not told. The link runs slower until they look. The
  2026-11-07 case went unwarned before ADR-0105 too; it was found by the waterfall.
* A program that read `metrics.rx_filter` finds it missing.
* A browser's `aether.filterDismissed` stays in its storage, unread.
* A reading of the filter comes back only if it is simple and robust, and passes
  `05-busy-band.md` §7, Q1 included.
