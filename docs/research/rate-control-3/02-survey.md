# Gate 1: the survey's update

The first survey (`docs/research/rate-control/02-survey.md`) and the second's update
(`docs/research/rate-control-2/02-survey.md`) stand. This adds only the two questions the third
review's brief asks (`00-brief.md` §5, Gate 1): how much weight a rate-adaptation scheme gives a
rate it has not measured, against the ones it has, and how long a try runs before its verdict. As
before, each source says how far it was read (§4). The sources were read on 2026-11-13 by a
reading pass told to cite only what it reached. The two claims the candidates lean on most,
Mercury's estimators and Minstrel-HT's inheritance, were then checked here against the source
files. Arithmetic of this review's own is marked as such.

## 0. Summary

* **Once a rate is measured, the others drop their prior.** Minstrel-HT gives a never-sampled rate
  nothing, or a faster sampled sibling's success probability, and one interval's probe replaces it.
  SampleRate starts a rate at its lossless time and gives that no weight after the first packet.
  The Intel driver's rate scaling treats an unmeasured faster rate as worth trying ("No data about
  high/low rates. Increase rate"). Aether's prior, worth two frames that a three-frame try must
  outweigh, is heavier than any of them.
* **Mercury met Aether's near miss and changed three things.** The open HF modem Mercury
  (Rhizomatica) keeps an optimistic prior worth two pseudo-frames, but only for a rung the SNR
  supports with 3 dB to spare: "a clean link climbs on the first probe". A clean probe gets a
  second round before it is judged, because "near its threshold one frame scores 1/2, under the
  rung below, and the session went straight back down". And a failed probe's wait doubles only
  while the rung is marginal and delivering under half: "backing off regardless cost a link that
  rose from 3 to 10 dB 17 %".
* **A three-frame try can only say "all clean" or "worse".** RRAA sizes its windows so that one
  loss stays under the threshold for going up. Between Aether's neighbouring rungs the break-even
  loss is 9–26 % (this review's arithmetic, from the rate ratios 1.10–1.36), and one loss in three
  frames is 33 %.
* **No source measured what a prior's weight should be.** The one SNR-informed prior found in code
  (BayesLA) is worth 100 frames, but it is held per channel report and refreshed with every new
  report. LTS samples pessimistically on purpose. Flat priors (Beta(1,1)) are optimistic for fast
  rates and paid for it: about twenty intervals of near-zero throughput at the start.
* **The doubled wait is Aether's alone in its form.** SampleRate excludes a rate that failed four
  times running for a fixed 10 s; ARDOP asks a mode that never worked for five ACKs in a row before
  trying it again; Mercury doubles only for marginal rungs that lost half or more. None doubles
  after a clean try that lost its verdict to the prior.

## 1. How much weight an unmeasured rate gets

* **Minstrel-HT** (Linux master `7e1392a4`, `net/mac80211/rc80211_minstrel_ht.c` and `.h`, the
  statistics, sampling and selection paths read; the inheritance lines checked here). A rate never
  sampled has a success probability of 0, and a rate under 10 % earns no throughput, so it cannot be
  chosen. In `minstrel_ht_update_stats`, walking each group from its fastest rate down, a rate with
  no attempts takes the best probability of a faster sampled rate in the same group
  (`if (mrs->att_hist) last_prob = max(last_prob, mrs->prob_avg); else mrs->prob_avg =
  max(last_prob, mrs->prob_avg);`); nothing crosses groups. A rate's first measurement is taken
  whole; after that a second-order filter of period 16 runs on each 50 ms interval's pooled success
  ratio. There is no minimum count: at the next update a sampled rate competes on its probability,
  capped at 90 %, so one clean probe of a faster rate beats a current rate at 90 % or better.
* **SampleRate.** Bicket's thesis (chapter 5, read in full) starts at the highest rate ("If no
  packets have been successfully acknowledged, return the highest bit-rate that has not had 4
  successive failures"), and one packet can switch it ("After the first 5.5 megabit packet required
  no retries, SampleRate … switched bit-rates"). FreeBSD's `sample.c` (main `acd270c8`, the named
  functions read) starts each rate's average at its lossless time; the plain average over the first
  four packets gives that start no weight once one packet is measured, then an average keeping 75 %
  of the old value. It switches at once during a link's first 20 packets and at most once a second
  after. A rate that failed four times running is not sampled for 10 s (`stale_failure_timeout`),
  a fixed time that never doubles.
* **Intel's driver rate scaling** (Linux `drivers/net/wireless/intel/iwlwifi/mvm/rs.c`, master
  `7e1392a4`, read). A rate's throughput counts only after 8 successes or 3 failures within its last
  62 attempts. Toward an unmeasured faster rate it is optimistic: "No data about high/low rates.
  Increase rate".
* **Mercury** (Rhizomatica/mercury master `333a11cf`, `datalink_arq/carousel.c`, the link-adaptation
  section read; the quotes here checked against the file). Unmeasured rungs are left out of the
  choice; one is probed when its lossless potential beats the best rung's measured goodput, unless
  the SNR reads more than 1.5 dB under its threshold. A measured rung's delivery comes from frame
  counts decayed by 0.8 a round ("memory of ~5 frames"), by one of two estimators: where the SNR is
  3 dB or more over the rung's threshold, 1 − (lost + ½)/(sent + 2), so one clean frame scores
  0.83; within 3 dB, (sent − lost)/(sent + 1), so one clean frame scores 0.5. Its reason for two:
  "an optimistic prior kept a dead QAM16C2 outscoring a working DATAC3 on the cliff for 100+ rounds,
  and a pessimistic one never climbed on a clean channel." A rung is dead after four frames lost in a
  row with nothing delivered (two when marginal), and the lowest dead rung is a ceiling.
* **Bayesian and Thompson-sampling link adaptation.**
  * BayesLA (Saxena and Jaldén, SPAWC 2020; the code read, the paper not reached): a Beta per
    modulation-and-coding scheme and channel report, its prior the AWGN error rate at the SINR the
    report implies, worth `prior_weight = 100` frames, with no forgetting.
  * LTS (Saxena, Tullberg and Jaldén, arXiv 2010.08651v2, full text): the prior is over the SINR
    and samples are taken pessimistically on purpose (the smaller of a sample and the mean), because
    "overestimating the channel SINR is even more detrimental". Its flat-prior comparison (UTS) had
    near-zero throughput for "approximately the first 20 time intervals".
  * Liu and Li (arXiv 1506.03378, abstract): Thompson sampling's regret grows as O(√(T/p)) with a
    bad prior and O(√((1−p)T)) with a good one, p the prior's mass on the truth.
* **For Aether.** Every Wi-Fi scheme read replaces its prior with the first measurement; Aether's
  keeps counting as two frames beside a try of three, which is the near miss of `00-brief.md` §1
  (G3). Mercury's split says where optimism is safe: a rung the SNR supports with room to spare.
  Aether's prior already rises toward 1 with the SNR's margin over a rung's AWGN point; its 3 dB cut
  is what holds it under that on a clean path (`01-evidence.md` §2, G1). No source gives a weight
  to copy: the weight has to be judged on Aether's own paths.

## 2. How long a try runs before its verdict

* **Minstrel-HT:** one probe, one try, per sampled rate; judged at the next 50 ms update.
* **SampleRate:** one packet.
* **RRAA** (Wong, Yang, Lu and Bharghavan, MobiCom 2006, §5.1 and its Table 3 read in full). It
  sends no probes; going up moves all traffic to the next rate for a window of 6–40 frames, sized so
  that one loss stays under the threshold for going up (1/ewnd < P_ORI). Its thresholds come from
  the rates' air times (P* = 1 − t(R)/t(R−)), and it decides early once the frames left in the
  window cannot change the outcome.
* **Intel's driver:** the whole traffic, until 8 successes or 3 failures.
* **Mercury:** a one-frame probe, then a second round of two frames before the choice judges the
  rung (only above the lower half of the SNR's doubt, where a frame that came "is more likely luck
  than the rung working"). A measured rung above the best is probed again every 8 polls; the wait
  doubles, up to 64, only while the rung is marginal and delivering under half: "backed off it stayed
  on DATAC4 for 42-46 frames where the build without the backoff was back on DATAC3 after 8".
* **ARDOP** (ardopcf `e27af046`, `src/common/ARQ.c`, the gear-shift functions read). Every frame is
  acknowledged on its own. Up needs the averaged quality over the mode's threshold and two ACKs in a
  row; down takes two NAKs, or one if the mode has not worked this session; a mode tried that never
  worked needs five ACKs in a row before it is tried again. It does not shift up when what is left
  fits in one frame of the current mode.
* **FreeDATA** (main `2b3f037d`): no try; the receiving station sets the level from the latest SNR
  against fixed thresholds every burst.
* **For Aether.** A try's length is set by what it has to tell apart. Three frames cannot see a loss
  rate under 33 %, and the break-even losses between Aether's neighbours are 9–26 %, so a try today
  can only find a rung clean or not (this review's arithmetic). That is enough on a strong path, where
  the rung above either decodes everything or does not, and it is why a clean try losing to the
  prior is the waste to remove. Mercury's probe and its confirming round together are three frames,
  Aether's length; what differs is that Mercury does not judge a clean probe on the prior. Putting the
  try's frames in one transmission (G6) has no direct counterpart: Wi-Fi tries cost microseconds, and
  Mercury's rounds are one transmission each.

## 3. Not reached

* The BayesLA paper (SPAWC 2020); only its code was read.
* Saxena's KTH thesis (2021) and the NetAI 2019 contextual-bandit paper (a search snippet of its
  abstract only).
* Krotov, Kiryanov and Khorov, IEEE Access 2020 (only through another paper's account).

## 4. Sources

| source | reading level |
|---|---|
| Linux `net/mac80211/rc80211_minstrel_ht.c`, `.h`, master `7e1392a4` | code: statistics, sampling and selection; the inheritance lines checked here |
| Linux `drivers/net/wireless/intel/iwlwifi/mvm/rs.c`, `constants.h`, master `7e1392a4` | code: window statistics and the rate action |
| J. Bicket, *Bit-rate Selection in Wireless Networks*, MIT MS thesis, 2005 | chapter 5 in full |
| FreeBSD `sys/dev/ath/ath_rate/sample/sample.c`, `.h`, main `acd270c8` | code: the functions named in §1 |
| S. H. Y. Wong, H. Yang, S. Lu, V. Bharghavan, "Robust Rate Adaptation for 802.11 Wireless Networks", MobiCom 2006 | §5.1 and Table 3 in full |
| Rhizomatica, Mercury, `datalink_arq/carousel.c`, master `333a11cf` | code: the link-adaptation section; the quotes checked here |
| ardopcf (pflarue/ardop) `src/common/ARQ.c`, master `e27af046` | code: the gear-shift functions |
| FreeDATA (DJ2LS/FreeDATA) `freedata_server/arq_session*.py`, main `2b3f037d` | code: the speed-level paths |
| V. Saxena, J. Jaldén, BayesLA code (vidits-kth/bayesla-link-adaptation) | code and README; the paper not reached |
| V. Saxena, H. Tullberg, J. Jaldén, "Reinforcement Learning for Efficient and Tuning-Free Link Adaptation", arXiv 2010.08651v2 (IEEE TWC 2022) | full text, proofs skimmed |
| V. Saxena et al., "Constrained Thompson Sampling for Wireless Link Optimization", arXiv 1902.11102v2 | §§1–4 |
| C.-Y. Liu, L. Li, "On the Prior Sensitivity of Thompson Sampling", arXiv 1506.03378 | abstract |
