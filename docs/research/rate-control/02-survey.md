# Gate 1: the survey

*The rate controller's review (`00-brief.md` §5), 2026-10-24. What cellular systems, Wi-Fi and HF
standards and modems do to choose a data rate, read for one question: what fits a half-duplex HF
ARQ link where one exploratory exchange costs 6–9 s of air.*

The sources were read in three passes (cellular and effective SNR; Wi-Fi; HF standards and
modems), each told to cite only what it reached and to say how much of it: the full text, a
section, the abstract, a secondary account. The reading level is given with every source in §8.
A claim that rests on a secondary account says so where it is made. Where this document draws a
conclusion of its own, it says so. Copies of what was read are not committed: most are other
people's copyright. VARA HF was looked for in its published documentation only, and none was
reached that says how it chooses its speed (§4.7).

## 0. Summary

* **The best of the others answer the brief's question.** The best-measured Wi-Fi rate
  controllers, the throughput literature on link adaptation with HARQ, STANAG 5066 Edition 4's
  advice (a speed for maximum throughput) and the newest open HF modem (Mercury's carousel)
  choose the rate that delivers the most data per unit of air. Cellular practice targets a fixed
  error rate (10 %) and corrects an SNR offset towards it (OLLA). The simplest schemes step on an
  error rate, and their evaluations record them oscillating or settling low.
* **With HARQ, the first copy should be allowed to fail often.** On block-fading channels the
  throughput-optimal first-transmission error rate is "quite large", and trends towards one as
  more copies may be combined (Wu and Jindal, §2.3). Without HARQ, the switch point between two
  rates is where the faster one's error rate reaches one less the ratio of their rates
  (Sassioui et al.): 10–36 % between neighbouring Aether rungs.
* **Aether's controller aims far lower than that, and climbs later still.** Its learned margin
  balances when about a quarter of six-frame bursts lose a frame, 4–8 % of frames (§2.2). It
  climbs only when the smoothed SNR clears the next rung's 10 %-error AWGN threshold by the
  margin (1.5 dB at least) and 1.5 dB more; on AWGN the next rung then loses nothing. This is the
  mechanism behind `01-evidence.md` §5 item 1: the climb stops below what the ladder decodes.
* **Exploration can be nearly free.** Minstrel probes with one try at the head of a retry chain
  that carries the packet anyway; SampleRate prunes candidates by their lossless air time; the
  unimodal bandits sample only the leader's neighbours. Aether has the pieces: data frames of
  one family take the same air, a header states how many frames follow (so a frame never
  detected is counted), and the Test's measurement frames exist (ADR-0077).
* **A frame never heard must count.** ARDOP's gear shift sees only ACKs and NAKs, and Aether's
  controller sees only frames detected: both are blind to the silent loss. Mercury scores
  silence as loss, and SoftRate distinguishes silent losses from collisions to step down.
* **The fading penalty is not one number.** MIL-STD-188-110C's tables put the gap between AWGN
  and ITU Poor at 5 dB for its slowest high rate and 10 dB for its fastest; Aether's own model
  puts it at 4–10 dB depending on rung and class. One learned margin over one AWGN table cannot
  fit every rung, which is why calibration per rate (CHARM, SGRA, Schulze's table) or
  measurement per rate (SampleRate, Minstrel, Mercury) replaced it elsewhere.
* **Ranking** (§6): first, choose by measured delivery per rung with an SNR-derived prior, and
  explore on frames that are sent anyway; second, give the receiver a soft per-frame reading
  that predicts the neighbouring rungs; third, if a threshold rule survives, derive its target
  from the rungs' rate ratios and HARQ, not from a fixed 10 %. Target-error stepping
  (Trinder/Gillespie, ARF/AARF) is last.

## 1. The link the survey reads for

What makes this link unlike Wi-Fi and cellular, in the terms the comparison needs
(`00-brief.md` §2):

| property | Aether | Wi-Fi | cellular |
|---|---|---|---|
| one decision's feedback | one exchange, 6–9 s (tone floor ~3× longer) | one frame, ~0.1–1 ms | one TTI, 0.125–1 ms |
| outcomes in a session | 13–300 exchanges, 80–1 800 data frames | thousands a second | thousands a second |
| rates | 15 (500 Hz) or 20 (2300 Hz) rungs, ×1.11–×1.57 apart | 8–100+ | 15–28 MCS |
| retransmission | HARQ, RV 0, 0, 2, 3, soft combining | ARQ, no combining | HARQ, IR |
| who decides | the receiver recommends, the sender follows | the sender | the base station, from the UE's CQI |
| two families | tone floor (5.36 s frames, constant envelope) and OFDM | one | one |
| silent loss | countable: the header states how many frames follow | invisible (no ACK) | invisible to OLLA |

The ladder's numbers (`tools/ladder_table.py`): neighbouring rungs differ in data rate by ×1.11
to ×1.57; the AWGN thresholds (10 % frame error, `tools/update_rate_table.py --fer 0.10`) are
1–4 dB apart within a family, with a gap of 5–8 dB where the floor meets the first OFDM rung.
The modelled 10 % points on ITU Good, Moderate and Poor sit 4–10 dB above AWGN's, by an amount
that varies from rung to rung and class to class (Poor's the least, most likely because its
larger delay spread gives the code more frequency diversity).

## 2. Cellular link adaptation

### 2.1 CQI and MCS selection (3GPP NR)

* **What it measures.** The UE reports "the highest CQI index which satisfies" the condition
  that one transport block "could be received with a transport block error probability not
  exceeding 0.1" (0.00001 for the URLLC table). TS 38.214 §5.2.2.1 and Table 5.2.2.1-2, read
  as a section.
* **How it decides.** The specification fixes only the reporting rule. How the base station
  maps the report to an MCS is left to the implementation.
* **Its steps.** The CQI table is designed with a constant SINR offset between entries, about
  1 dB (Pocovi et al., full text); CQI n+1 needs about 2 dB more than CQI n on block Rayleigh
  (Tran and Eltawil, passages read).
* **For Aether.** A fixed 10 % target is the cellular convention, set for steps 1–2 dB apart
  and many outcomes a second. Aether's rungs are coarser in places (up to 8.5 dB where the
  2300 Hz floor meets OFDM on Moderate) and its outcomes six or so per exchange.

### 2.2 Outer-loop link adaptation, and where Aether's controller differs

* **The rule.** OLLA adds an offset to the reported SINR: after each first transmission, the
  offset rises by Δup on an error and falls by Δdown on a success. Its mean settles where the
  error rate is T = 1/(1 + Δup/Δdown), the familiar Δdown = Δup·T/(1−T); at T = 0.1, Δup is
  9·Δdown (Blanquez-Casado et al., EURASIP JWCN 2016, full text; Pocovi et al., full text).
  Nakamura, Awad and Vadgama's original (WPMC 2002) was not reached.
* **Its steps.** They must stay under a bound set by the slope of the error curve: Δdown below
  0.52 dB at 15 dB on Rayleigh with T = 0.1 (Blanquez-Casado et al.). Small steps converge slowly
  and deliver more once converged (2.98–3.00 bps/Hz at Δdown 0.001–0.01 dB, 2.76 at 0.5 dB, all
  meeting the target). From a random start on AWGN, OLLA (T = 0.1, Δ = 0.5 dB) "settles … after
  a few tens of transmissions" (Saxena et al., arXiv 2010.08651, sections read).
* **eOLLA.** The same paper replaces the one-bit error with a modelled instantaneous error rate
  from the SNR estimate, Δ = Δdown·(B/T − 1), every TTI: it halves OLLA's loss at large steps and
  gains about 15 % in an LTE-A simulation with HARQ.
* **Fading and HARQ.** OLLA knows nothing of HARQ: it is fed the first transmission's ACK.
  A Bell Labs system study ran it at "a block error rate target of 20 percent on the first
  transmission" with IR and up to four transmissions (Balachandran et al., one passage read).

**Aether's controller is OLLA-like, with five differences** (`model/aether_model/link/rate.py`;
this review's reading):

1. **The offset moves per burst, not per frame.** A burst that lost frames widens the margin
   (by 1.5 dB, or towards what the failed rung's reading implies, at most 3 dB; by the share
   lost when under half of a burst failed, ADR-0084), and resets the clean count.
2. **Its steps are not derived from a target.** The decay after a failure waits three clean
   bursts, then gives back 0.25 dB, then twice the last step each clean burst, at most 1 dB
   (ADR-0007). The target this implies was computed for this survey by simulation (the margin's
   rules alone, each frame lost independently with probability p, the rung held; the script is
   in the review's scratch work):

   | frames a burst | where | per-frame error the margin balances at | bursts with a loss |
   |---|---|---|---|
   | 6 | above the first OFDM rung | 8.1 % | 40 % |
   | 6 | at the first OFDM rung (every loss counts in full) | 4.2 % | 23 % |
   | 4 | above / at | 9.9 % / 6.3 % | 34 % / 23 % |
   | 2 | either | 12.2 % | 23 % |

3. **Climbing needs more than the target.** The smoothed SNR must clear the next rung's 10 %
   AWGN threshold plus the margin (never under 1.5 dB) plus 1.5 dB of hysteresis, for at least
   one clean burst, at most two rungs at a time. On AWGN the 500 Hz OFDM modes go from 10 %
   lost to none of 30 within about a decibel (the modes on the ladder,
   `bench/baselines/phy_fer_500.csv`), so the climb waits until the next rung would lose
   nothing. The margin targets 4–8 %; the climb targets 0 %.
4. **It is one offset for every rung and both families.** The modelled fading penalty differs
   from rung to rung by 4–6 dB on one class (500 Hz: 6.5–10.6 dB on Good, 4.1–10.1 on
   Moderate), so one learned number is right for at most a few rungs.
5. **It cannot see a frame that was never detected,** nor a burst that HARQ rescued: the
   former counts for nothing, the latter as clean (`01-evidence.md` §5).

### 2.3 How much error the first copy should accept when copies combine

This is the question the brief marked most important, and the literature answers it plainly.

* **Wu and Jindal, "Coding versus ARQ in fading channels"** (GLOBECOM 2009, arXiv 0904.0226,
  full text). Rayleigh block fading, no channel knowledge at the sender. With plain ARQ the
  goodput-optimal error rate falls only slowly with SNR, and "choosing ε = 0.1 leads to
  near-optimal performance"; insisting on ε = 0.001 costs about 10 dB with two fading blocks a
  codeword and 2 dB with ten. **With incremental-redundancy HARQ** of at most M rounds, "the
  initial rate should be chosen such that the first-round outage ε1 is quite large, and for
  larger values of M the optimizer actually trends towards one". When the fading is slow
  against the feedback loop, the problem is "trivial": send just below the instantaneous
  capacity.
* **Wu and Jindal, "Performance of hybrid-ARQ in block-fading channels"** (IEEE Trans. Commun.
  58(4), 2010; arXiv 0811.4191v3, full text). The rate that meets an outage ε after M rounds is
  "roughly constant with respect to ε" with HARQ, and falls sharply as ε → 0 without it. Chase
  combining, which adds SNR rather than mutual information, is "far inferior to IR at moderate
  and high SNR's" and "performs reasonably well at low SNR".
* **Sassioui et al., "HARQ and AMC: friends or foes?"** (arXiv 1606.05177, model, sections III–V
  and conclusions read). With channel knowledge, the throughput-optimal boundary between rates
  R_{l−1} and R_l is where R_l(1 − PER_l) = R_{l−1}(1 − PER_{l−1}), so PER_l ≈ 1 − R_{l−1}/R_l at
  the switch: 0.5 for a doubling, "contrasted with … PER_t = 10⁻¹". On slow fading HARQ adds
  little on top of AMC; on fast fading at high SNR it can cost, because the HARQ cycle ignores
  what the channel now says.
* **Andreotti, Lottici and Giannetti** (EURASIP JWCN 2018:5, abstract, assumptions and results
  read) choose the MCS of every round to maximise expected goodput, combining the bit-level
  effective SNR across rounds: about 4 dB over a mutual-information mapping and 7.5 dB over EESM
  at 4 bit/s/Hz. They assume perfect channel knowledge each round.

**For Aether** (this review's reading):

* Between neighbouring rungs (×1.11–×1.57) the switch point without HARQ lies at a 10–36 %
  error rate on the faster rung, and HARQ moves it higher. The controller's targets of 4–8 %
  (the margin) and 0 % (the climb) are far below it.
* Two corrections pull the other way. Aether's RV sequence 0, 0, 2, 3 makes the second copy a
  repeat (Chase-like), weak at high SNR; and an exchange costs a header, turnarounds and an
  answer that the models above do not charge. The optimum first-copy error is therefore lower on
  the fast OFDM rungs than on the floor, and lower than the textbook HARQ optimum. It is not
  near zero.
* Moderate, Poor and flutter fade within a frame and between exchanges (copies one exchange
  apart see independent fades): Wu and Jindal's regime. Good and slow QSB are nearer the slow
  regime where the current reading should pick the rung, close to the waterfall.

### 2.4 Effective SNR mapping (EESM, MIESM)

* **What it is.** SINR_eff = Φ⁻¹[(1/N) Σ Φ(SINR_n)] over the subcarriers, then the frame error
  rate is read off that MCS's AWGN curve. EESM: SINR_eff = −β ln[(1/N) Σ exp(−SINR_n/β)], β
  fitted per MCS and block length; MIESM averages the constellation's mutual information instead
  (IEEE 802.16m Evaluation Methodology 08/004r4 §4–§5.1.1; 3GPP TR 25.892 Annex A, both read as
  sections). β is fitted by least squares from link simulations; TR 25.892 Table 24 gives 1.49–
  1.69 for QPSK and 3.36–7.68 for 16-QAM.
* **HARQ in the method.** Chase: sum the per-subcarrier SINRs across copies. IR: accumulate
  mutual information over all copies as one codeword. "The link adaptation algorithm should be
  optimized to maximize the performance at the end of the HARQ process" (802.16m EMD §5.1.1).
* **Accuracy.** An error of 1 dB in the effective SNR can shift the predicted error rate by 1.5
  decades. A receiver choosing its MCS from per-subcarrier mutual information came within 1.7 dB
  of the throughput bound and needed 9.5 dB less SNR than one choosing from the noise variance
  alone, but 4.5–5.5 ms of feedback delay degraded the prediction "drastically", and the authors'
  remedy was an outer loop (Jensen et al., IEEE TVT 2010, preprint read in full).
* **For Aether.** The receiver has per-carrier noise and SNR (`per_carrier_noise`, ADR-0043), so
  an effective SNR per rung is within reach for OFDM. With 6–9 s between exchanges it predicts
  the last frame, not the next: averaged over recent frames, it is a prior, not a decision. The
  tone rungs need a non-coherent mapping of their own. The link bench's fading pipe already uses
  EESM (`aether_model/link/fading.py`), calibrated per frame class.

### 2.5 Slow feedback and expensive probes

* **Satellite and NTN** (3GPP TR 38.821 §6.2.3 and §6.4.1, read). Aged channel reports cost
  28–38 % of throughput at 3 km/h non-line-of-sight; prediction recovered about 10 %, averaging
  nothing. A DVB-S2 adaptive-coding study switches ModCod on SNR thresholds with "a fixed value
  of hysteresis", the up hysteresis higher than the down (Csurgai-Horváth 2018, one passage
  read). Where the round trip exceeds the channel's variation, choosing by the propagation
  *state* (a Markov model) rather than the last reading is proposed (Tarchi et al. 2015,
  introduction read).
* **Latent Thompson sampling** (Saxena et al., arXiv 2010.08651, sections II, V and VI read).
  One Bayesian distribution over the SINR, updated from each ACK/NACK through an offline link
  model, sampled pessimistically, widened each step in proportion to the Doppler; the MCS
  maximises expected throughput. Up to twice OLLA's throughput on a 30 km/h channel; equal to
  OLLA with reports every 80 ms. From a flat prior it spent about 20 intervals near zero
  throughput while it explored.
* **For Aether.** The lesson is a single latent SNR shared by all rungs (one outcome informs
  every rung), seeded from the call's reading, so that exploration is not paid rung by rung.

## 3. Wi-Fi rate adaptation

Wi-Fi's controllers face many cheap outcomes; ours face few dear ones. What transfers is
structure, not constants. The per-frame constants below are each converted to Aether's scale
(six data frames a transmission, 6–9 s an exchange); the conversions are this review's.

### 3.1 Counting successes: ARF, AARF, AMRR, Onoe

* **ARF** (as described by Lacage et al. and Bicket; Kamerman and Monteban 1997 not reached):
  up after 10 successes in a row or a timer, down after one or two failures; the first frame
  after a step up is a probe that "must succeed or the rate is immediately decreased".
* **AARF** (Lacage, Manshaei and Turletti, MSWiM 2004, full text): a failed probe doubles the
  successes needed to try again, from 10 up to 50. **AMRR** (FreeBSD `ieee80211_amrr.c`) applies
  the same backoff to periods of more than ten frames, capped at 15. **Onoe** (FreeBSD `onoe.c`)
  raises the rate after ten credit periods of at least ten frames with retries under 10 %.
* **Their evidence.** Simulation against RBAR (AARF); testbeds where ARF and AARF "perform
  poorly" (Bicket).
* **For Aether.** None weighs air time per byte, so none can tell a lossy fast rate from a clean
  slow one. At our scale AARF's 50 successes are about nine transmissions (a minute), AMRR's
  15 periods 3.5–4.5 minutes, Onoe's 10 credits 2–3 minutes a rung: the whole session for a
  ladder. Loss-triggered methods "underselect when they require consecutive successful packets
  to increase their transmission rate" on fading channels (Camp and Knightly, §3.4): Aether's
  symptom. One idea to keep: AARF's doubling backoff after a failed climb.

### 3.2 Measuring delivery: SampleRate, Minstrel and Minstrel-HT

* **SampleRate** (Bicket, MIT MS thesis 2005, full text; FreeBSD `sample.c`). It measures each
  rate's average transmission time per packet delivered, "including time required to recover
  from losses": every try's air is charged. Each packet goes at the rate with the lowest average
  over the last ten seconds. Every tenth packet samples a random rate "that may do better", and
  a rate is not eligible if its *lossless* transmission time exceeds the current rate's average
  (or four packets in a row failed at it). FreeBSD budgets sampling as a share of air time, 10 %.
  Testbeds of 45 indoor and 38 outdoor nodes: "always performs very close to the best static
  bit-rate". Bicket names its cost: a link sampling a rate that needs four retries "might get
  only half the throughput". One failed probe can keep a rate out for long: RRAA's authors saw
  "25 lossless probes … 250 frame transmissions" before SampleRate switched (§3.3).
* **Minstrel-HT** (Linux `net/mac80211/rc80211_minstrel_ht.c`, master and v5.10, read; commit
  messages b1103d256704 and 80d55154b2f8). Success probability per rate per statistics window,
  smoothed (since 2019 a second-order filter of period 16, chosen because a plain moving average
  "comes at the cost of introducing lag"). Throughput = probability / air; a rate below 10 %
  scores zero, and probability is capped at 90 % "to account for collision related packet error
  rate fluctuation". Four best-throughput rates and a best-probability rate form a retry chain.
  "Sudden death": a best rate with more than 30 attempts and under a quarter succeeding is
  dropped at once. An unsampled rate inherits the probability of a faster sampled one.
  **Probing is nearly free:** the probe goes as the first try of the chain, once; if it fails,
  the chain still carries the packet. Since 2021 candidates come in a fixed mix, "every 6
  attempts are split into 3x INC, 2x JUMP, 1x SLOW" (the next faster rate, a random faster one,
  one between the robust and the best), because the old design "needs way too many probing
  packets". No published measurement was found; the developers report a "very small delta"
  against the best fixed rate.
* **For Aether.** The metric is the brief's: air per byte delivered, every copy charged. Two
  structural ideas transfer directly. A probe can be *one frame of a transmission that is sent
  anyway*: within a family every data frame takes the same air (ADR-0085), so a frame at a
  neighbouring rung costs at most its payload's difference, and a failed one goes again at the
  current rung. And a candidate is worth trying only if its *lossless* delivery beats the
  current rung's measured one. The time constants must be counted in frames and exchanges: ten
  seconds is one exchange.

### 3.3 Thresholds from rate ratios: RRAA

* **RRAA** (Wong, Yang, Lu and Bharghavan, MobiCom 2006, full text) measures the loss ratio over
  a short window of 6–40 frames and never probes. Its thresholds come from the rates
  themselves: P*(R) = 1 − Throughput(R−)/Throughput(R) is the loss at which R stops beating the
  rate below; it goes down above α·P* (α = 1.25) and up below P_MTL(R+)/β (β = 2). Up to 35.6 %
  over SampleRate and 143.7 % over ARF with mobile clients, 3.8 % over SampleRate with static
  ones, on a programmable access point.
* **For Aether.** The principle is RRAA's gift: the thresholds are *derived* from the ladder's
  rate ratios, not tuned. With equal-length frames, P* = 1 − bytes(r−1)/bytes(r); a ×1.33 step
  gives P* = 25 %. Its weakness is Aether's: climbing needs low loss at the current rung, which
  a fading path rarely shows.

### 3.4 SNR-based, and calibrated: RBAR, CHARM, SGRA, Camp and Knightly

* **RBAR** (Holland, Vaidya and Bahl, MobiCom 2001, full text): the receiver measures SNR on the
  RTS and picks the fastest rate whose bit error rate is at most 1E-5. Structurally this is what
  Aether does now. Bicket's measured counter-evidence: at an SNR of 10 dB, delivery of 1 Mbit/s
  packets ranged "from 2% to 100%".
* **CHARM** (Judd, Wang and Steenkiste, MobiSys 2008, full text) calibrates each rate's SNR
  threshold online, per receiver: "if a given rate is failing at SINRs above its threshold, it
  probably should be increased … if a rate is succeeding below its threshold, it probably should
  be decreased". It beat AMRR, Onoe and SampleRate, most of all under mobility.
* **SGRA** (Zhang et al., INFOCOM 2008, full text) models each rate by the SNRs of 10 % and 90 %
  delivery, calibrated online, and picks the highest delivery × rate. It names Aether's failure
  exactly: a high rate whose threshold is set too high "will not be updated since no frames will
  be sent with that rate. So it becomes a sort of chicken-egg problem." The cure is a forced
  probe of the next rate, at most once a period. Up to 30 % over ARF, RRAA and HRC without
  interference.
* **Camp and Knightly** (MobiCom 2008, abstract, introduction and §3 read): on a WARP testbed,
  loss-triggered protocols underselect on fast fading, SNR-triggered ones overselect when the
  coherence time is short, and "trained SNR-based protocols outperform loss-based protocols".
* **For Aether.** Calibrating the SNR-to-rung mapping per rung from outcomes binned by reading
  is the direct alternative to one global margin, and SGRA's forced probe is the direct answer to
  a climb that never tries the rung above.

### 3.5 Soft information: SoftRate, effective SNR, MiRA

* **SoftRate** (Vutukuru, Balakrishnan and Jamieson, SIGCOMM 2009, full text). The receiver
  computes every frame's bit error rate from the decoder's soft outputs, "even when the packet
  has no bit errors", and predicts the other rates from it (BER at least ten times higher a rate
  up). Its thresholds depend on the error-recovery scheme: a scheme that resends parity accepts
  a hundred times the BER of plain frame ARQ. "Three consecutive silent losses indicate a weak
  signal", told apart from collisions by a postamble. Twice SampleRate and RRAA in a GNU Radio
  prototype and trace-driven simulation.
* **Effective SNR** (Halperin, Hu, Sheth and Wetherall, SIGCOMM 2010, full text): from the
  channel state, per modulation, ρ_eff = BER⁻¹(mean of BER(ρ_s)); its transitions are under
  2 dB for most links against up to 10 dB for RSSI. Online it beat SampleRate by "a small
  (5–10%) margin"; SampleRate's habit of dropping a rate for retries was "an important factor
  that gives it short-term adaptability".
* **MiRA** (Pefkianakis et al., MobiCom 2010, full text): 802.11n's single- and dual-stream
  modes break the monotone relation between rate and loss, and controllers assuming it "got stuck
  at lower rates". MiRA probes two neighbours in the current mode and one in the other, with an
  exponential backoff on failed probes.
* **For Aether.** Every frame, decoded or not, could say how close it came to its cliff (the LDPC
  decoder's margin, or an effective SNR per rung); then most exploration on the OFDM rungs would
  be unnecessary. Aether's header count already gives SoftRate's silent-loss signal without a
  postamble. And Aether's two families are MiRA's two modes: ADR-0087's NVIS path had tone rung 4
  decoding nothing while OFDM decoded a quarter of its frames, a broken monotone relation the
  controller must expect at the floor's edge.

### 3.6 Bandits

* **ORS, G-ORS, SW-ORS** (Combes, Proutiere, Yun, Ok and Yi, INFOCOM 2014 preprint arXiv
  1307.7309v3, full text; the IEEE TMC 2019 version's abstract). If success probabilities fall
  with the rate and throughput is unimodal in it, the regret's lower bound has "at most two
  terms, irrespective of the number of available rates": only the leader's neighbours need
  samples. The algorithm plays the leader regularly and otherwise the neighbour with the best
  KL-UCB index; a sliding window follows a changing channel.
* **Thompson sampling.** Restricted to the leader and its neighbours (Paladino et al., AAAI 2017,
  algorithm and claims read), with Beta posteriors; "Bayesian MAB algorithms dramatically
  outperform frequentist ones". Constrained Thompson sampling (Gupta, Eryilmaz and Srikant,
  INFOCOM 2019, full text) draws a joint sample constrained to be monotone, needing no
  unimodality "since such assumptions are hard to justify in practice", and allows any prior
  whose update is exact.
* **Discounted and sliding-window UCB** (Garivier and Moulines, arXiv 0805.3415, full text):
  within a log factor of the lower bound under abrupt changes. With 250 exchanges and five
  changes the sliding window would be about 33 exchanges, four minutes (this review's
  arithmetic).
* **For Aether.** The bounds are asymptotic and the algorithms start by playing every rate:
  15–20 exchanges, two to three minutes, at our scale. What transfers is the structure — sample
  only the leader's neighbours, with a prior from the SNR so the start is not a round-robin — and
  the reward priced in bytes per second of air.

## 4. HF standards and modems

### 4.1 MIL-STD-188-110 (A to D)

* **Nothing on how to choose a rate.** The waveform announces its rate and interleaver in the
  preamble ("autobaud", "critical in developing an efficient (ARQ) protocol"); the receiver must
  "deduce the data rate and interleaver setting". For ARQ the standard defers to STANAG 5066
  (110C §4.4; 110C with Change 1 searched in full; 110D read in part).
* **Its evidence is its performance tables**, which show the fading penalty growing with the
  rate. SNR for a bit error rate of 1E-5 (110C Table C-XVII):

  | data rate, bit/s | AWGN, dB | ITU-R Poor, dB | penalty, dB |
  |---|---|---|---|
  | 3 200 | 9 | 14 | 5 |
  | 4 800 | 13 | 19 | 6 |
  | 6 400 | 16 | 23 | 7 |
  | 8 000 | 19 | 27 | 8 |
  | 9 600 | 21 | 31 | 10 |

* **For Aether.** One margin over an AWGN table cannot be right for every rung. Stating the rate
  on the air, as the chips and the header do, is all the standard asks.

### 4.2 MIL-STD-188-141 (2G link quality; 3G HDL and LDL)

* **2G link quality analysis** (141B App. A, Table A-XLII, read): an ALE response carries a
  mean-SNR score that "may be used as a first approximation to setting data rates", from 50–
  75 bit/s at −6 to −3 dB up to 4 800 bit/s at 21–27 dB.
* **3G HDL and LDL** (141B App. C, read): no rate adaptation within a datagram. The sender
  stipulates a traffic type (HDL 24/12/6/3, LDL 512/256/128/64) and the receiver echoes it;
  HDL is "best suited to … large datagrams under good to fair" conditions. **Adaptation comes
  from incremental redundancy:** each retransmission carries the next of four encoded blocks of
  a rate-1/4 code, so the effective code rate is "variable: 1/1 to 1/4" (§C.5.1.5.5). 141D
  replaces this appendix by a reference to STANAG 4538, which was not reached.
* **For Aether.** In HDL, HARQ *is* the rate adaptation: the channel decides how many copies a
  packet needs. A controller that ignores what combining delivers judges the wrong quantity.

### 4.3 STANAG 5066

* **What was reached.** The Edition 4 draft's core text (2021, public; its Annex C was not
  reached), Isode's Edition 4 overview (full), Isode's white papers (through a summarising fetch,
  not verbatim), and Schulze's account of Edition 3's data rate change and the published FER
  algorithms (§4.4; the originals were not reached).
* **Edition 3: the receiver decides.** The receiving node "sets the TX data rate of the remote
  node for the next transmission interval"; the sender may refuse only a request to go faster
  (Schulze). Connections were to start at 300 bit/s with short interleaving (Edition 1.2
  §C.6.4.1, as quoted in US patent 7,092,402, which instead seeds the first rate from an ALE link
  quality exchange).
* **Edition 4: the sender decides, advised.** "Ed3 uses a Data Rate Change process which is
  controlled by the receiving server. Ed4 replaces this with a Data Rate Selection process, where
  the data rate is chosen by the sender, taking into account information provided by the
  receiver" (Isode). The advice includes the speed for maximum throughput (which "will likely lead
  to significant data loss followed by retransmission"), the speed for low latency, a probe speed
  with a likelihood, and frame-error estimates for the optimum and up to seven speeds below; the
  sender weighs its queue and the service it needs, and may choose "conservative speeds when
  there is not much data".
* **The published FER algorithms** (through Schulze): halve the rate above 50 % frame error and
  double it at 0 % (Trinder and Brown 1999); over the air the best pair was up below 20 %, down
  above 50 %; per-rate thresholds for STANAG 4539 from 50 %/10 % at 3 200 bit/s to 5 % down at
  9 600 (Trinder and Gillespie 2001); "Always be conservative when selecting data rates" (Nieto
  2002).
* **Measured on skywave** (Kille, Isode, Nordic HF 2016, full text). Second-by-second SNR traces
  of six good and three poor runs, scored against a per-second "perfect" oracle and a per-slot
  "achievable" one:

  | link | method | throughput |
  |---|---|---|
  | good, 120 s transmissions | FER stepping (Trinder/Gillespie) | 59–76 % of achievable |
  | good, 120 s transmissions | mean SNR read against a table | 88–95 % of achievable |
  | good | the best fixed speed | 73–80 % of perfect |
  | poor, 400 miles, 5 s transmissions on 5 s of SNR | SNR table | 64–72 % of perfect |
  | poor, 120 s transmissions | SNR table | 30–46 % of perfect |
  | poor | the best fixed speed | at most 47 % of perfect |

  On good links an averaging window of 60–240 s was sensible and 10 s too short; on poor links
  short transmissions on short averages won. Isode's 2014 measurements (summarised) saw swings of
  15–20 dB over tens of seconds to minutes, losses in blocks rather than scattered, and "the speed
  with negligible loss is much slower than the optimum".
* **For Aether.** The receiver should advise more than one rung (throughput, low latency, a
  probe), the sender should weigh its queue, and the averaging should follow how variable the
  path is. Aether's 6–9 s exchanges put weak, variable paths in Kille's "short transmissions win"
  regime. FER stepping is the weakest method measured.

### 4.4 Schulze's data rate change algorithm (RapidM, 2005)

* **Source**: Schulze, *Design and implementation of a STANAG 5066 DRC algorithm for high data
  rate autobaud waveforms*, M.Eng, University of Pretoria, 2005 (chapters 1–7 read).
* **The rules**: the fastest rate under a fitted BER = 1E-5 line for CCIR Good; move by the BER's
  exponent, at most two up and three down; force 75 bit/s below −2 dB; restart on large SNR
  changes. **The redesign keeps a per-rate BER estimate table**, seeded with CCIR Good values,
  averaged per rate from measurements, shifted by whole rates when the SNR moves by 3 dB; it
  probes only into a rate with no estimate, and tracks the highest that passes.
* **Evidence**: commercial RM6 modems on a simulator: about 400 bit/s more than Trinder's
  algorithm on every channel, 9 oscillations against 49.7, acquisition in 19 intervals against 49.
  Its own caveat: "the assumption of a 3 dB difference between data rates, while being true for an
  AWGN channel is not a very good assumption for a CCIR Poor … and definitely not for a CCIR Good
  channel". A follow-on (Sequeira, IST Lisbon 2017, abstract and §5.4 read) reported +392 %
  average goodput on a 300 km field link with prediction guards added.
* **For Aether.** A per-rung memory, re-anchored when the SNR moves, probing only where nothing
  is known: the model-based form of "estimate each rung, take the best".

### 4.5 PACTOR

* **PACTOR-III** (ITU-R M.1798-2 Annex 3 §1, SCS's description, read in full): the receiver
  steps one level at a time — "CS4 demands an increase of the speed to the next higher level. CS5
  acts as a NAK asking for a repetition … and at the same time for a reduction of the speed to the
  next lower level" — with "Memory-ARQ to smooth the transitions between speed levels". No
  thresholds are published; maximum throughput needs 14 dB.
* **PACTOR-4**: SCS's description and manuals were not reached (the publisher's site redirects);
  a listing shows MAXUp and MAXDown commands, but not what they do.

### 4.6 ARDOP

* **Sources**: ardopcf (pflarue/ardop at e27af04), `src/common/ARQ.c` and its neighbours, and
  Muething's ARDOP Specification revision 2.0 (2017), read. Studied only.
* **What it measures**: the receiver reports a constellation quality of 0–100 on every ACK and
  NAK ("a Quality value of 60 or above is typically required").
* **How the sender decides** (`Gearshift_9`): quality smoothed half and half; down after two
  NAKs (one if the mode "has never worked"); up when the smoothed quality passes the mode's
  threshold (`{80, 84, 84, 75, 79}` on the 500 Hz list) after at least two ACKs, but not if the
  rest fits the current frame, and not soon into a mode that "has been tried before, and
  immediately failed" — "till we get at least 5 successive acks". The thresholds were calibrated
  by finding where the faster mode decodes four in five and averaging the slower mode's quality
  there. A session starts at the middle of the list ("a frame type of moderate speed and
  robustness") or the slowest.
* **Its blind spot is Aether's**: one frame an exchange, and the gear shift runs only on an ACK
  or a NAK; a frame never detected gives it no evidence.
* **For Aether.** Thresholds calibrated in the *current* mode's measured quality, not SNR, are
  cheap; so is remembering a rung that was tried and failed.

### 4.7 VARA HF

Only VARA's own published material was searched. What was reached (the author's blog post of
2017-09-03) gives the waveform (OFDM, 2 400 Hz, 52 carriers at 37.5 baud, up to 5 629 bit/s at
14.5 dB SNR in 4 kHz) and nothing about how a speed is chosen; the manuals could not be fetched.
Nothing in this survey's design reasoning comes from VARA.

### 4.8 FreeDATA, Mercury, Clover

* **FreeDATA** (DJ2LS/FreeDATA at 2b3f037, `arq_session*.py`, read): the receiver picks the
  highest level whose minimum SNR the last decoded frame's SNR meets (−10, 0, 3, 8.5 dB), with no
  memory; the sender drops to level 0 after two burst timeouts. Nothing protects it against a run
  of bad readings.
* **Mercury's carousel** (Rhizomatica/mercury at 5ef8b7f, `docs/CAROUSEL-*.md`, `ARQ.md`,
  `FADE-CLIFF-DECISION.md`, `SPEED-REGRESSION-FINDINGS.md`, constants checked in
  `datalink_arq/carousel.c`; GPL-3.0, studied for its ideas). Its earlier ladder used OLLA, a
  per-link SNR offset with ±1 dB hysteresis and fade-cliff patches, which its documents now call
  "mostly history". The carousel is receiver-driven:
  * per-rung decaying counters, sent = 0.8·sent + frames, lost = 0.8·lost + frames·loss;
  * the rung is the argmax of goodput = rate × delivery × air/(air + overhead), with the overhead
    measured (6.31 s a round, 2.55 s at the floor);
  * it probes only rungs whose full-delivery potential beats the best, re-probes a measured rung
    every eight polls, probes above the best where the SNR would start a session, and treats four
    frames lost in a row as a dead verdict, re-probed after 60 s, doubling to eight minutes;
  * a round that brought nothing is repeated once unscored on a rung that has delivered, and
    otherwise "the silence is scored against the mode";
  * the first rung is the one the connect SNR supports, sent as a one-frame probe; HARQ is off.

  Its evidence: on NVIS, goodput rose with the mode's speed despite heavy erasure (one mode
  delivered 80 % of its frames for 1.9 B/s, a faster one 7 % for 7.0 B/s); a delivery-driven
  ceiling it tried lost 23 %; on the air the carousel against the old ladder was "a tie".
* **Clover-II and Clover-2000** (Wickwire, Bernock and Levreault, DCC 1998, full text): the
  receiver measures "the amount of error-correcting capacity used … the number of erroneous data
  blocks and the phase dispersion" and sends it at each turnaround; the transmitter follows; the
  operator sets a bias (fast, normal, robust) for the session. The share of error-correcting
  capacity used is a margin reading even on frames that decode.

## 5. What the survey says of Aether's controller

These are the survey's conclusions, item by item against `01-evidence.md` §5; they are inputs to
Gate 2, not designs.

1. **The climb stops below what the ladder decodes** because the climb's condition is a 0 %
   target (§2.2 item 3), where the literature puts the switch at 10–36 % between Aether's
   neighbours, higher with HARQ (§2.3). Every measured method that chooses by delivery climbs
   until delivery falls (SampleRate, Minstrel, Mercury).
2. **The first OFDM rungs are chosen where the floor's top carried more** because the controller
   compares SNR against two families' AWGN thresholds with one margin, where the families' fading
   penalties and their behaviour under combining differ (§2.2 item 4; MiRA's two modes; Mercury
   scores each rung on its own delivery).
3. **The learned margin walked a strong session down** because it is one number moved by
   bursts, sticky after a failure, and nothing tells it which rung's frames failed for what
   reason. Per-rung calibration (CHARM, SGRA, Schulze) or per-rung measurement (SampleRate,
   Minstrel, Mercury) does not carry one rung's losses to the others.
4. **A third of rung 7's frames never detected** count for nothing today; the header's count
   makes them countable, as SoftRate and Mercury count silence.
5. **Frames rescued by combining** make a burst clean today; with HARQ the quantity to maximise is
   delivery after combining, per unit of air (802.16m EMD §5.1.1; HDL; Wu and Jindal).
6. **The start** is one reading of a floor frame and two steps in hand. The literature seeds from
   the link's measurement and then measures (Kille's SNR table as a prior, STANAG 5066 Edition 4's
   advice, Mercury's one-frame probe at the connect SNR's rung), rather than holding back.

## 6. Ranking: what fits a link where one exploratory exchange costs 6–9 s

Each is this review's judgement, from the sources above.

1. **Choose by measured delivery per rung, with a prior from the SNR, and explore on frames that
   are sent anyway.** Per rung: bytes delivered after combining, divided by all the air its
   frames took, every copy charged and a frame never detected charged as air spent (SampleRate's
   metric; Mercury's argmax; RRAA's thresholds as the derived boundaries). Unmeasured rungs take a
   prior from the reading and the AWGN table less a class penalty (Kille; Schulze's table), with a
   few pseudo-frames of weight so one failure does not banish a rung (RRAA's lesson about
   SampleRate). Explore only the leader's neighbours (the unimodal bandits), plus one edge across
   the floor's boundary (MiRA), by sending one frame of a normal transmission at the neighbour
   (Minstrel's single try) or by the measurement frames a short transmission leaves room for
   (ADR-0077); back off a failed climb exponentially (AARF, Mercury's dead verdict). Count in
   frames, decay over minutes (Minstrel's period, the sliding-window bounds). It removes the
   learned margin, the hysteresis, the dwell, the decay schedule, `partial_share`, the edge rule
   and the re-seed; what it keeps of the SNR is the prior.
2. **A soft per-frame reading at the receiver that predicts the neighbouring rungs** (SoftRate's
   BER from the decoder; an effective SNR per rung from the per-carrier noise, EESM/MIESM). If it
   predicts the neighbours within about 2 dB, as effective SNR did in Wi-Fi, most exploration on
   the OFDM rungs becomes unnecessary, and the tone rungs keep item 1's measurement. Instrument it
   first: it is worth building only if the field and harness recordings show it predicting.
3. **If a threshold rule survives, derive it.** The switch points from the rate ratios (RRAA's
   P*, Sassioui's 1 − R_{l−1}/R_l), raised for HARQ, as an eOLLA-style soft update toward that
   target rather than a 10 % AWGN point plus a margin plus headroom; calibrated per rung from
   outcomes (CHARM, SGRA), with SGRA's forced probe so a rung never stays untried.
4. **A latent-SNR Bayesian tracker** shared by all rungs (Saxena et al.), its uncertainty widened
   by the path's Doppler class: sample-efficient, but more machinery than item 1 for the same
   decision, and its gains were shown where feedback is fast.
5. **The receiver's advice set and HARQ's share of the work** (STANAG 5066 Edition 4, HDL): they
   shape what a receiver should report (a throughput rung, a safe rung, per-rung loss) and say
   that combining, not the controller, should absorb small fades.

**Last**: target-error stepping (ARF/AARF/AMRR/Onoe, Trinder/Gillespie) and a memoryless SNR table
(FreeDATA). Every evaluation read records the former oscillating or settling low, and nothing
protects the latter from a run of bad readings.

For Gate 2 this ranking names three candidates beside the simplified current controller
(`00-brief.md` §6): item 1 (delivery-based, with SNR prior and free exploration), item 3 (derived
thresholds with per-rung calibration), and item 1 with item 2's soft reading as its prior once the
reading is shown to predict. Where the decision lives (the receiver, which counts what arrived,
or the sender, which sees the answers and its queue) is open in every source: STANAG 5066 moved it
from the receiver to the sender between editions, Mercury keeps it at the receiver, Wi-Fi at the
sender.

## 7. Not reached

* Nakamura, Awad and Vadgama (WPMC 2002), the original OLLA paper; Duran et al. (2015) and
  Sarret et al. (2015) on OLLA, Park, Daniels and Heath (GLOBECOM 2015), Frederiksen and Kolding
  (VTC 2002); Tsai and Soong on MIESM.
* Kamerman and Monteban (1997), ARF's original; Gupta et al.'s INFOCOM 2018 paper.
* STANAG 5066 Annex C of any edition; the Trinder/Brown, Trinder/Gillespie and Nieto papers (only
  through Schulze); STANAG 4538 and HDL+; FED-STD-1052; DTIC reports (refused).
* SCS's PACTOR-4 description and the PTC manuals; the VARA HF manuals.

## 8. Sources

Reading levels: *full* (the whole text), *sections* (as cited), *abstract*, *secondary* (through
another source's account), *summarised* (through a fetch that summarised the page, not verbatim),
*source* (code at the stated version).

**Cellular, effective SNR, slow feedback**

* 3GPP TS 38.214 V18.11.0, §5.2.2.1 — sections.
* 3GPP TR 25.892 V6.0.0, Annex A.4.3.2, A.4.4, A.4.6 — sections.
* 3GPP TR 38.821 V16.2.0, §6.2.3, §6.4.1 — sections.
* IEEE 802.16m-08/004r4 Evaluation Methodology (2008), §4–§5.1.1 — sections.
* Blanquez-Casado, Gomez, Aguayo-Torres, Entrambasaguas, "eOLLA: an enhanced outer loop link
  adaptation for cellular networks", EURASIP JWCN 2016:20 — full.
* Pocovi et al., IEEE Access 2018 — full.
* Tran and Eltawil, Wireless Commun. Mob. Comput. 2014, doi 10.1002/wcm.2292 — passages.
* Saxena, Tullberg, Jaldén, arXiv 2010.08651v2 — sections II, V, VI.
* Balachandran et al., Bell Labs Tech. J. 18(2), 2013 — one passage.
* Wu and Jindal, "Coding versus ARQ in fading channels: how reliable should the PHY be?",
  GLOBECOM 2009, arXiv 0904.0226v2 — full.
* Wu and Jindal, "Performance of hybrid-ARQ in block-fading channels: a fixed outage probability
  analysis", IEEE Trans. Commun. 58(4), 2010, arXiv 0811.4191v3 — full.
* Sassioui, Jabi, Szczecinski, Le, Benjillali, Pelletier, "HARQ and AMC: friends or foes?",
  arXiv 1606.05177 — model, sections III–V, conclusions.
* Andreotti, Lottici, Giannetti, EURASIP JWCN 2018:5 — abstract, assumptions, results.
* Brueninghaus et al., PIMRC 2005 — abstract.
* Jensen, Kant, Wehinger, Fleury, "Fast link adaptation for MIMO-OFDM", IEEE TVT 59(8), 2010 —
  full (preprint).
* Csurgai-Horváth, Int. J. Satell. Commun. 2018, doi 10.1002/sat.1270 — one passage.
* Tarchi, Corazza, Vanelli-Coralli, Int. J. Satell. Commun. 2015, doi 10.1002/sat.1133 —
  introduction.

**Wi-Fi**

* Lacage, Manshaei, Turletti, "IEEE 802.11 rate adaptation: a practical approach", MSWiM 2004 —
  full.
* FreeBSD `sys/net80211/ieee80211_amrr.c`, `sys/dev/ath/ath_rate/onoe/onoe.c`,
  `sys/dev/ath/ath_rate/sample/sample.c`, main branch, 2026-10-23 — source.
* Bicket, "Bit-rate selection in wireless networks", MIT MS thesis, 2005 — full.
* Linux `net/mac80211/rc80211_minstrel_ht.c`/`.h` (master, 2026-10-23, and v5.10), legacy
  `rc80211_minstrel.c`, commits b1103d256704 and 80d55154b2f8 — source; the legacy documentation
  page — full.
* Wong, Yang, Lu, Bharghavan, "Robust rate adaptation for 802.11 wireless networks", MobiCom
  2006 — full.
* Holland, Vaidya, Bahl, "A rate-adaptive MAC protocol for multi-hop wireless networks", MobiCom
  2001 — full.
* Judd, Wang, Steenkiste, "Efficient channel-aware rate adaptation in dynamic environments",
  MobiSys 2008 — full.
* Zhang et al., "A practical SNR-guided rate adaptation", INFOCOM 2008 — full.
* Camp and Knightly, "Modulation rate adaptation in urban and vehicular environments", MobiCom
  2008 — abstract, introduction, §3.
* Vutukuru, Balakrishnan, Jamieson, "Cross-layer wireless bit rate adaptation", SIGCOMM 2009 —
  full.
* Halperin, Hu, Sheth, Wetherall, "Predictable 802.11 packet delivery from wireless channel
  measurements", SIGCOMM 2010 — full.
* Pefkianakis et al., "MIMO rate adaptation in 802.11n wireless networks", MobiCom 2010 — full.
* Combes, Proutiere, Yun, Ok, Yi, "Optimal rate sampling in 802.11 systems", arXiv 1307.7309v3 —
  full; IEEE TMC 18(5), 2019 — abstract.
* Paladino et al., "Unimodal Thompson sampling for graph-structured arms", AAAI 2017, arXiv
  1611.05724 — algorithm and claims.
* Gupta, Eryilmaz, Srikant, "Link rate selection using constrained Thompson sampling", INFOCOM
  2019 — full.
* Saxena, Gonzalez, Stoica, Tullberg, Jaldén, arXiv 1902.11102v2 — full.
* Garivier and Moulines, "On upper-confidence bound policies for non-stationary bandit problems",
  arXiv 0805.3415v1 — full.
* Perchet et al., batched bandits, Ann. Stat. 2016 — abstract.

**HF**

* MIL-STD-188-110C with Change 1 (2012) — searched in full, sections read; MIL-STD-188-110D
  (2017) — in part.
* MIL-STD-188-141B (1999), Appendices A and C — sections; MIL-STD-188-141D (2017), Appendix C —
  section.
* NATO STANAG 5066 Edition 4 draft, core text (v1.6, 2021) — full; Annex C not reached.
* Isode, "STANAG 5066 Edition 4" overview — full; Isode white papers S5066-EP4, "Optimising
  STANAG 5066 parameter settings for HF and WBHF", and the 2014 intermediate-term variation
  measurements — summarised.
* Kille (Isode), "Optimizing applications and data links for HF radio ITV: can you ride the
  wave?", Nordic HF 2016 — full.
* US patent 7,092,402 B1, Smith et al. (Rockwell Collins), 2006 — full.
* Schulze, M.Eng dissertation, University of Pretoria, 2005 — chapters 1–7.
* Sequeira, MSc thesis, IST Lisbon, 2017 — abstract, §5.4.
* ITU-R M.1798-2 (02/2021), Annex 3 §1 — full section.
* ardopcf, pflarue/ardop at e27af04 — source; Muething, ARDOP Specification rev. 2.0 (2017) —
  full.
* FreeDATA, DJ2LS/FreeDATA at 2b3f037 — source.
* Mercury, Rhizomatica/mercury at 5ef8b7f — documents and source.
* Wickwire, Bernock, Levreault, "On-air measurements of CLOVER II and CLOVER 2000 throughput",
  DCC 1998 — full; a 1993 Clover article reposted on a bulletin board — secondary.
* rosmodem.wordpress.com, VARA HF announcement, 2017-09-03 — page text.
