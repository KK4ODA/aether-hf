# Gate 1: the survey's update

The first survey (`docs/research/rate-control/02-survey.md`) stands. This adds only the two
questions the second review's brief (`00-brief.md` §4, Gate 1) asks that it did not: where a link
decides its rate when both ends can measure, and how a link handles a path whose multipath
refuses a whole waveform family. As before, each source says how far it was read.

## 0. Summary

* **The sender decides; the receiver reports.** In the three systems where both ends measure and
  the design has been revisited, the decision sits at the sender and the receiver's word is an
  input. Cellular: the receiver reports a CQI (the highest entry that would meet 10 % block
  error), and the transmitter picks the MCS and corrects it with OLLA on its own ACK/NACK record.
  802.11n: the receiver may recommend an MCS (MCS feedback, optional), and the sender may ignore
  it; the Linux rate controller read here uses the sender's own transmit statistics alone.
  STANAG 5066 moved from the receiver deciding (Edition 3) to the sender deciding on the
  receiver's information (Edition 4). Aether's D1 has the same shape.
* **No source refuses a family from delivery statistics alone.** Minstrel-HT groups its rates by
  guard interval, streams and width, keeps every rate's own account, samples across groups toward
  faster rates, and lets an unsampled rate inherit a faster sampled rate's probability. It never
  refuses a group as such, because a sample costs one try of one frame. Where a try costs a whole
  exchange, as in Aether, the inference Minstrel does not need (within a family, if the most
  robust member fails, the faster ones fail too) has to be made.
* **Where a family is refused, it is refused from a measurement of the channel.** OFDM systems
  that adapt their guard interval choose it from the measured delay spread (rule of thumb: a
  guard two to four times the spread), and an HF patent chooses between OFDM and single-carrier
  modes from the delay spread measured on received signals. Aether already measures the
  dispersion on its floor frames (ADR-0087's echo reading). Its limits refuse the fast tone kinds
  at 500 Hz but not OFDM, whose 5 ms prefix is shorter than the 7 ms echo of ITU NVIS.

## 1. Where the rate is decided when both ends can measure

* **Cellular** (the first survey §2.1–2.2, read there). The receiver measures and reports the
  highest CQI meeting a 10 % error target. The base station decides the MCS, and OLLA corrects
  the reported SINR toward the target from the first transmissions' ACK/NACK. The receiver's
  report is a measurement framed as a recommendation; the decision and its correction are the
  transmitter's.
* **802.11n MCS feedback** (CWNP's link adaptation page and a 2014 HT Control field note, read in
  summary; patents through search snippets). The HT Control field carries an MCS request (MRQ)
  from the sender and an MCS feedback (MFB) from the receiver, tagged by a sequence identifier.
  Responding is optional (a station advertises whether it does). The sender may use the feedback;
  a patent notes the standard gives the sender no reference for how to interpret it. Linux's
  `rc80211_minstrel_ht.c` (master, read for this update) takes the transmit status of each
  frame and nothing from the receiver beyond it.
* **STANAG 5066 Edition 4** (the core text, draft v1.5 of 2021-04-06 as hosted by Isode in 2024,
  read in full; Isode's Edition 4 overview, read). "To achieve Data Rate Selection, as described in
  Annex C, the sender needs to control the modem's sending speed and associated parameters such
  as interleaver." The receiver monitors SNR and other characteristics to provide
  "recommendations". The sender chooses its initial speed, and may choose conservative speeds
  when there is little to send. Annex C was not reached again: Isode says the promulgated
  Edition 4 (May 2022) is had through a NATO representative, and the draft it hosts is the core
  text without annexes.
* **For Aether.** Aether's receiver already reports most of what the sender needs: every
  acknowledgement says which frames arrived (base and bitmap), and every granted control frame
  states the SNR the other station was read at (`GrantFrame.snr_db`, ADR-0021). Its recommended
  rung (`GrantFrame.rung`) is a second controller's opinion. Every system above treats that kind
  of word as advisory, and the evolving ones moved the decision to the side that sees both the
  outcomes and the queue. What only the receiver knows is whether a frame not acknowledged was
  never detected or detected and not decoded, and what it measured of the path beyond one SNR
  (the echo reading, the family offset). A report of those, not a rung, is what D1 would need
  from it.

## 2. A path that refuses a waveform family

* **Minstrel-HT's groups** (`rc80211_minstrel_ht.c` and `.h`, master, read for this update; the
  first survey §3.2 read the same code for the rest). Rates are organised in groups by streams,
  guard interval and channel width, and a rate's account is its own. Sampling happens at most
  every 20 ms (`MINSTREL_SAMPLE_INTERVAL` = `HZ / 50`) in a fixed mix of six (`INC, JUMP, INC,
  JUMP, INC, SLOW`). INC "flips through groups and picks the first group rate that is faster than
  the highest currently selected rate"; JUMP picks random faster rates; SLOW a rate between the
  most robust and the best, skipped once its probability is above 95 %. A rate under 10 %
  success scores no throughput, and "sudden death" drops a best rate after more than 30 attempts
  with under a quarter succeeding. A group of short-guard rates that the delay spread defeats is
  never refused as a group: each of its rates loses on its own record, one cheap try at a time.
* **Guard intervals chosen from the delay spread** (patents and abstracts through search
  summaries, not read in full). OFDM systems with a choice of guard interval pick the shorter one
  when the measured delay spread allows, the guard set at about two to four times the spread.
  Later patents choose a client's guard by comparing the physical rate each candidate would
  achieve from the measured channel impulse response. The choice comes from a measurement of
  the channel, not from losses.
* **An HF mode chosen from the delay spread** (US 8,897,394 and its continuation 9,420,594, "Methods
  and apparatus for adaptively selecting a communications mode in high frequency systems", through
  a search summary; the text was refused twice). A dual-mode HF system computes the delay spread
  (the time between the first and last multipath component) from received signals, and chooses
  OFDM when it is high and single carrier when it is low. Its OFDM's guard is evidently sized for
  long spreads; Aether's is not (5 ms), so the same principle points the other way here.
* **What HF paths do** (Angling, Cannon, Davies, Willink, Jodalen and Lundborg, *Radio Science*
  33(1), 1998, abstract read). On four oblique high-latitude paths, the spreads exceeded 5 % of the
  time ranged from 1 to 11 ms in multipath and 2 to 55 Hz in Doppler, and modem availability at
  0 dB SNR ranged from 64 to 100 % where a mode was supported. A 5 ms OFDM prefix is not a
  rare-case limit on such paths.
* **HF standards.** STANAG 4415's 75 bit/s mode exists for severely degraded channels with large
  Doppler and multipath spread, and STANAG 4539's lever against fading is the interleaver's length
  (sigidwiki's summaries, read). MIL-STD-188-141's link quality analysis lists SNR, interference,
  multipath delay spread and Doppler spread among the parameters a link measures (US 4,633,411, a
  search summary); 4G ALE uses link quality to order frequencies (Isode, *Not using ALE*, Nordic HF
  2022, a search summary). No public rule mapping those measurements to a waveform was found.
* **For Aether.**
  1. The family is the unit a path refuses on dispersion: the floor's 40 ms tones survive a 7 ms
     echo that breaks the four-tone kinds' 10 ms symbols and outruns OFDM's 5 ms prefix
     (ADR-0087's bench). The echo reading is the measurement the sources above choose a mode
     from, and Aether has it from the first frame of a session (the call or acceptance).
  2. ADR-0087's limits were set by comparing first-copy delivery rung by rung, so at 500 Hz they
     cover the four-tone kinds and no OFDM rung: on ITU NVIS QPSK ½ decodes 4–6 frames in 16
     against tone4x100-51's 8–9 (`bench/baselines/echo.csv`), and its higher rate wins on paper.
     Gate 0 (`01-evidence.md` §3) shows the cost: C1's first choice on such a path went to OFDM
     rung 6.
  3. Minstrel's inheritance runs downward (an unsampled rate takes a faster sampled rate's
     probability). Aether needs the converse too, and both are the same assumption: within a
     family the success probability does not rise with the rate. ADR-0087 already assumes it
     ("monotone within a family").

## 3. Not reached

* STANAG 5066 Edition 4 Annex C, in any version (above).
* The texts of US 8,897,394 and 9,420,594 (USPTO and Google Patents refused), and MIL-STD-188-141D.
* Any measurement of 802.11n MCS feedback in use.

## 4. Sources

* Linux, `net/mac80211/rc80211_minstrel_ht.c` and `rc80211_minstrel_ht.h`, master as of 2026-11-02.
* NATO STANAG 5066 Edition 4 draft, core text v1.5 (2021-04-06), whole v1.6 (2021-08-27), hosted
  by Isode at `isode.com/wp-content/uploads/2024/04/Main.pdf`; Isode, "STANAG 5066 Ed4" whitepaper
  page.
* CWNP, "Link Adaptation"; mrncciew, "CWAP – HT Control Field" (2014).
* US 8,897,394; US 9,420,594; US 4,633,411 (search summaries only).
* M. J. Angling et al., "Measurements of Doppler and multipath spread on oblique high-latitude HF
  paths and their use in characterizing data modem performance", *Radio Science* 33(1), 1998,
  doi:10.1029/97RS02206.
* sigidwiki, "STANAG 4415", "STANAG 4539".
