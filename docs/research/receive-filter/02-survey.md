# The receive-filter estimate: Gate 1

*2026-11-08. Two halves: what published work and other programs say (§1–§3), and what Aether's
own receiver says about a cut — why one costs (§4), and what the other station's frames show of
it (§5). §6 lists what this suggests for Gate 2, and §7 asks the author two things.*

## 1. Estimating a noise spectrum's shape while its level swings

The literature is from speech enhancement and spectrum monitoring: a noise floor whose level moves
and which other signals cover part of the time.

* **Minimum statistics** (Martin 2001): each band's minimum of a smoothed periodogram over 1–3 s,
  corrected for its bias. It assumes the signal is absent for part of the window and the noise
  steadier than the signal; it tracks a rising floor about a second late and has about twice a
  plain estimator's variance (Gerkmann and Hendriks 2012; Cohen 2003).
* **Quantile estimation** (Stahl, Fischer and Bippus 2000): each band's quantile (often the
  median) over time. It holds while each band is only noise for at least that share of the time;
  one quantile for every band is its known weakness (Bonde et al. 2005).
* **IMCRA** (Cohen 2003): a recursive average steered by a presence probability from the minima,
  which keeps updating while signals are weak.
* **Spectrum-occupancy work** (Iwata et al. 2019; FCME, Vartiainen et al.) is closest to Aether's
  case: a floor whose shape depends on frequency and whose level moves. FCME removes the bins
  over a threshold computed from the rest, look by look, and is reported to work with signals on
  up to 85 % of the band.
* **What matters for a shape.** A gain that scales every bin together (an AGC) scales every bin's
  quantile alike, so it cannot bend a shape by itself. What bends one is an additive floor the gain
  does not scale (the audio chain's own noise), which dominates exactly the looks a low quantile
  picks, and signals that sit on some bins more than others. The model to estimate is
  `|Y|² = g(t)·|H(f)|²·N + F`. The 10th percentile's bias (9.8 dB under the mean for Gaussian noise)
  is the same in every bin and cancels in a shape; with 120 looks its spread is about ±1.1 dB a bin,
  the median's ±0.5 dB.

## 2. How a filter's edges are measured

* **ITU-R SM.1836** (receiver IF filters): the −6 dB bandwidth, swept with a generator at the
  antenna, read to 0.1 dB; the shape factor is the −60 dB width over the −6 dB width.
* **ITU-R SM.443 and SM.328** (x dB bandwidth): x dB under a 0 dB reference at the highest
  spectral line, and the measurement needs at least x + 5 dB of signal over what lies under it for
  under 10 % error. So an edge at −6 dB drawn from band noise needs that noise 11 dB over the
  audio chain's floor, and confirming a cut 20 dB deep needs 25 dB.
* **802.11 spectral flatness** judges each carrier against the mean over the inner carriers, not
  against the peak or a fixed frequency.
* **Amateur bench practice** (K7RLD, WB8YYY; WSJT-X's *Measure reference spectrum*): band noise or
  an antenna disconnected, averaged a minute, AGC off; the sound card's and audio stages' own
  response is in the result.

## 3. What other programs tell the operator

None of the programs read measures the radio's passband or warns that it is cut or moved:

* **WSJT-X**: use the widest USB filter, up to about 5 kHz; set band noise to about 30 dB on its
  meter with AGC off; *Flatten* corrects a slope or curvature in the display and "does not expect a
  steep filter edge within the displayed passband". No warning of a narrow or moved passband, no
  clipping alarm.
* **fldigi**: a diamond shows red when the level is too high, and the waterfall red bands.
* **ardopc** prints the input's peak every 10 s and asks for peaks within ±20000 of ±32767.
* **FreeDATA**: receive level mid-range and not clipping, the receive path as flat as possible.
* **JS8** (an operators' page): a light band with darker sides on the waterfall means the filter
  is too narrow; set about 3 kHz.
* **Mercury, Pat**: nothing on either.
* **VARA HF**: its developer's site says it is "designed for operation within a SSB bandwidth of
  2400 Hz"; the quick guides could not be opened, and nothing published was found on measuring or
  warning about the passband.

On clipping: counting samples at full scale finds clipping in the codec; clipping in the radio's
own audio stages, below digital full scale, shows in the amplitude histogram's tails (Aleinik et
al. 2014). Hard clipping spreads a noise band's power outside it (Van Vleck and Middleton 1966): a
clipping receiver fills the cut it is being asked about. Published level advice is coarse: no
program gives a dBFS target for band noise. For Gaussian noise 1.5 % of samples at full scale (the
mobile on Saturday) means full scale only 7.7 dB over the noise's RMS.

The sources and how much of each was read are listed at the end.

## 4. Why a cut costs: Aether's receivers assume flat noise

Gate 0 left two questions (`01-evidence.md` §10): why a 20 dB cut costs as much over a deep chain
floor as over a shallow one, when a radio's filter cuts the band's noise with the signal, and why
the bottom edge costs more than the top. `tools/bench_cut.py` sends each rung's data frames through
a cut applied to signal and noise alike, over a floor 50 dB down, at the rung's AWGN threshold
plus 3 dB, and decodes them with the model's `Modem.decode_buffer`. Frames decoded of 30:

| 2300 Hz | uncut | top 10 dB | top 20 dB | bottom 10 dB | bottom 20 dB | Saturday's |
|---|---|---|---|---|---|---|
| rung 1, tone-36 | 30 | 30 | 30 | 30 | 30 | 30 |
| rung 3, tone50-75 | 30 | 30 | 30 | 30 | 30 | **6** |
| rung 5, tone100-153 | 30 | 30 | 30 | 30 | 30 | **0** |
| rung 7, OFDM | 30 | 30 | 30 | 30 | 30 | 30 |
| rung 10 | 30 | 30 | 30 | 30 | 30 | 27 |
| rung 13 | 30 | 30 | 30 | 30 | 30 | **0** |
| rung 16 | 30 | 30 | 30 | 30 | 30 | **0** |
| rung 19 | 30 | 30 | 30 | 30 | 30 | **0** |

| 500 Hz | uncut | top 10 dB | top 20 dB | bottom 10 dB | bottom 20 dB | Saturday's |
|---|---|---|---|---|---|---|
| rung 1, tone-36 | 30 | 30 | **0** | 30 | **0** (of 20) | 30 |
| rung 2, tone4x100-51 | 30 | 30 | **0** | 30 | 22 | 30 |
| rung 3, tone4x100-75 | 30 | 30 | **0** | 30 | **1** | 30 |
| rung 4, tone4x100-93 | 30 | 30 | **4** | 30 | **1** | 30 |
| rungs 5, 8, 11, 14, OFDM | 30 | 30 | 30 | 30 | 29–30 | 30 |

* **A cut at the very edge costs the receiver nothing**: 10 and 20 dB at either edge of the
  2300 Hz signal leave every rung at 30 of 30 with the same SNR reading. The harness's costs for
  those cuts (`01-evidence.md` §8) are the session's course and the measure's step, not decodes.
* **A cut across the band costs the receiver a great deal.** Saturday's passband (ending near
  1.6 kHz) takes the fast tone rungs and every OFDM rung from 13 up; at 500 Hz the 20 dB cuts,
  whose corner lands past the channel's centre, take every tone rung. With 10 dB more SNR they
  decode again (rung 13: 8 of 8 at threshold + 13 dB), and rung 19 keeps an error floor (5 of 8 at
  + 13 and + 23 dB): the loss is systematic, not noise.
* **The mechanism, in the OFDM receiver** (`aether_model/phy/rx.py`, and the port's `rx.rs`): the
  noise is one figure per symbol, averaged over all the pilot carriers (`sigma2_sym`), and each
  data carrier is charged that noise over its channel estimate (`nv = sigma2_sym / |h|²`). A
  radio's filter cuts the band's noise with the signal, so a carrier in the cut has a small `|h|`
  *and* a small noise; the receiver charges it the band's average noise, calls it weak and all but
  discards it. ADR-0043's per-carrier factor only ever raises a carrier's noise, so it cannot give
  one back. Saturday's cut put about 40 % of the carriers into the skirt.
* **Confirmed** on a scratch copy of the model with the noise read per pilot carrier (each pilot
  carrier's residual power over the frame, interpolated across the data carriers): Saturday's cut,
  threshold + 3 dB, rung 13 0 → 20 of 20, rung 10 27 → 20 of 20, rung 19 0 → 9 of 12 (8 of 8 at
  + 13 dB); uncut frames decode as before (20 of 20, 12 of 12). Not in the tree.
* **The tone receiver** makes the same assumption by its construction: one noise level for all
  its tones (`sync_noise`, `symbol_metrics`), and the detector weighs a sync tone against the mean
  of the frame's other tones. A slope across the tones, noise and all, makes the loud tones' noise
  stand for the faint tones'. Not yet tested with a per-tone noise.
* **So**: the deep floor did not help on the harness because the floor was never the cost; the
  receivers' flat-noise assumption was. The bottom-versus-top difference is not the receiver's at
  2300 Hz (both edges decode alike); at 500 Hz both 20 dB cuts take the tone rungs alike, and the
  harness's 0.30 against 0.87 is left unexplained on three draws.

## 5. The other station's frames as a witness

`tools/passband_witness.py RECORDING SIDECAR` decodes again every frame the station decoded and
reads, for an OFDM frame, its channel estimate per carrier, and for a tone frame the energy at each
tone it sent (the decoded payload says which). Each frame's levels are taken against its own
median, so the sender's level and the path's loss drop out; the median over frames is printed by
100 Hz.

| Recording | Frames | 1.3 kHz | 1.6 kHz | 1.7 | 1.8 | 1.9 | 2.0 | 2.2 | 2.5 | 2.6 kHz |
|---|---|---|---|---|---|---|---|---|---|---|
| mobile, 2026-11-08 00:54Z (shift left) | 17 tone | +0.9 | −1.5 | −6.7 | −12.3 | −17.7 | −22.6 | −31.3 | | |
| mobile, 01:10Z (shift left) | 30 tone | +0.9 | −1.9 | −5.2 | −11.0 | −17.2 | −21.9 | −29.7 | | |
| mobile, 2026-11-07 15:06Z | 3 OFDM | −0.3 | +0.4 | +0.1 | −0.5 | +0.4 | 0.0 | −1.0 | −7.3 | −11.3 |
| mobile, same session | 11 tone | −0.7 | −0.5 | −0.5 | −2.5 | −1.6 | −2.3 | −1.8 | | |
| gateway's FTDX10, 01:10Z | 21 tone | +1.1 | −0.3 | +1.0 | +0.9 | −0.8 | −1.3 | −1.8 | | |

* **Saturday's cut shows in one frame.** The 00:54Z session's single fast-tone data frame
  (tone100-105: tones from 700 to 2200 Hz) draws the whole skirt, 7 to 31 dB down from 1.7 to
  2.2 kHz, and that frame came in the session's first exchanges; the session's seven frames of
  tone100-153, the same tones faster, all failed. The noise profile of the same
  session (`01-evidence.md` §3) agrees within a few decibels: two witnesses, one answer.
* **It measures the whole chain**: the sender's transmit filter, the path's fading and this
  radio's response. On 2026-11-07 the gateway's OFDM frames at the mobile fell 7 and 11 dB at 2.5
  and 2.6 kHz: the mobile's response then (its noise profile fell 6 dB at 2.4 kHz) or the gateway's
  transmit filter, which a second sender would tell apart. The mobile's frames at the gateway read
  within 3.5 dB from 0.7 to 2.2 kHz, with no cut.
* **Where it reaches**: OFDM frames cover the whole signal (360–2640 Hz at 2300 Hz); the fast tone
  kinds 700–2300 Hz; the slow tone kinds and every 500 Hz tone frame only 1300–1700 Hz. On Saturday
  at 2300 Hz the link never sent OFDM to the mobile, so the tone frames were the only witness there.
* **When**: only in a session, from the first frames it decodes; the noise is the witness when
  idle. Fading moves and a filter stays, so a few frames' median is the reading.

## 6. What this suggests for Gate 2

1. **The noise profile, made robust and judged right**: each look against its own in-band level,
   a median over looks, the chain's floor read where no radio passes audio (above about 4 kHz) and
   only looks whose band noise stands well over it trusted, edges at −6 dB from the mean over the
   signal's inner band, and the judgement the response at the signal's two edges with the side
   named, at a line between 10 and 20 dB (Gate 0 §8).
2. **The frames as a second witness**: OFDM channel estimates and tone levels, the median over a
   session's frames, over the span each kind covers.
3. **Clipping as its own reading**, and a veto on the passband reading while it lasts; shown where
   Compact shows it.
4. **The receivers' flat-noise assumption** (§4): outside this review's scope as written. Fixed,
   a cut that leaves the band's noise over the chain's floor would cost little, and the warning
   would matter mostly where a cut pushes the signal under that floor.
5. **Busy bands**: FCME-like removal of occupied bins within each look instead of waiting for
   whole quiet looks, so an FT8 band reads within seconds.

## 7. For the author

1. **The receivers' flat-noise assumption.** Fixing it is a receiver change (the model first,
   then the port, its vectors and the harness), with its own ADR, and §4 says it would take most
   of what a cut like Saturday's costs. Should it be done, as separate work, and before this
   review's Gate 3, so the warning's line is measured on the receiver that will ship?
2. **Where the warnings show.** Both the passband note and the clipping line are on the Status tab,
   and Compact hides the clipping line. Is a warning that shows in every view, Compact included,
   what you want (the session banner's place, or the header)?

## Sources

| Source | Reading |
|---|---|
| Martin 2001, *Noise power spectral density estimation based on optimal smoothing and minimum statistics*, IEEE TSAP 9(5) — [dblp](https://dblp.org/rec/journals/taslp/Martin01) | abstract; Martin's 2003 [slides](https://medi.uni-oldenburg.de/download/paper/Entwickler-Forum_2003/RAiner%20Martin2.pdf) in full |
| Gerkmann and Hendriks 2012 — [PDF](https://uol.de/f/6/dept/mediphysik/ag/speech/download/paper/gerkmann_unbiasedMMSE_TASL2012.pdf) | introduction |
| Cohen 2003, IMCRA — [PDF](https://numana.uni-koeln.de/sites/kunoth/user_upload/Mathematical_Foundations_Data_Analysis/SS19/Materials/IMCRA.pdf) | full text |
| Stahl, Fischer and Bippus 2000, quantile-based noise estimation — [dblp](https://dblp.uni-trier.de/rec/conf/icassp/StahlFB00.html) | abstract; method via Bonde et al. 2005 ([PDF](https://www.isca-archive.org/nolisp_2005/bonde05_nolisp.pdf), full) |
| Iwata et al. 2019, noise floor estimation for spectrum occupancy — [repository](https://livrepository.liverpool.ac.uk/3034132/) | abstract |
| ITU-R SM.1836 — [PDF](https://www.itu.int/dms_pubrec/itu-r/rec/sm/R-REC-SM.1836-0-200712-I!!PDF-E.pdf) | full |
| ITU-R SM.443-4 — [PDF](https://www.itu.int/dms_pubrec/itu-r/rec/sm/R-REC-SM.443-4-200702-I!!PDF-E.pdf) | full |
| ITU-R F.1487 — [PDF](https://www.itu.int/dms_pubrec/itu-r/rec/f/R-REC-F.1487-0-200005-I!!PDF-E.pdf) | the model's section |
| K7RLD, [measuring an IF filter](https://www.qsl.net/k7rld/Measuring%20an%20IF%20Filter.htm); WB8YYY, [IF filter analysis](https://qsl.net/wb8yyy/spectro.html) | pages |
| MathWorks, [802.11ac transmitter measurements](https://au.mathworks.com/help/wlan/ug/802-11ac-transmitter-measurements.html) | page |
| Coleri et al. 2002, channel estimation in OFDM — [PDF](https://ptolemy.berkeley.edu/projects/ofdm/ergen/docs/tmp/ofdm.pdf) | part |
| WSJT-X 2.5.4 user guide — [copy](https://www.VK5PAS.com/uploads/1/3/9/8/13982788/wsjtx-main-2.5.4_en.pdf) | the relevant sections |
| fldigi [beginners' guide](https://www.sourceforge.net/p/fldigi/wiki/beginners); ardopc [notes](https://www.cantab.net/users/john.wiseman/Documents/ARDOPC.html); FreeDATA [audio tuning](https://wiki.freedata.app/en/usage/tuning); SCARCS [JS8 page](https://scarcs.ca/_export/xhtml/links/js8); [Mercury](https://github.com/Rhizomatica/mercury) and [Pat](https://github.com/la5nta/pat) READMEs | pages |
| VARA's developer site — [rosmodem](https://rosmodem.wordpress.com/) | page (the quick guides would not open) |
| Aleinik et al. 2014, clipped fragments — [page](https://ntv.ifmo.ru/en/article/10352/DETECTION_OF_CLIPPED_FRAGMENTS_IN_ACOUSTIC_SIGNALS.htm); Van Vleck and Middleton 1966 — [index](https://manuscript.isc.ac/Inventory/49/878266.htm) | abstracts |

The survey was gathered by a helper session from these sources; where it read only an abstract or
a summary, the table says so, and nothing here rests on a detail it did not read.
