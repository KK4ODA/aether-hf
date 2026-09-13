# ADR-0004 — Peak-to-average power reduction

Status: **accepted** (2026-09-13, roadmap P2-4)
Supersedes nothing. Amends the transmitter only; the air interface of ADR-0002 is unchanged.

## Context

OFDM adds 57 independently modulated carriers, so its envelope is nearly complex Gaussian
and its peaks are large: the raw waveform measures **9–10 dB PAPR**
(`bench/baselines/papr.csv`). This costs real link margin, because an SSB transmitter is
driven at a fixed *peak* — the operator raises drive until ALC just starts acting — so the
average power that actually reaches the far end is peak power minus PAPR. It is also the
mechanism behind the field complaint recorded as `COMMUNITY-CONCERNS.md` #8, "high ALC
spikes when transmission starts".

The tempting conclusion is that every dB of PAPR removed is a dB of link gain. It is not,
and getting the accounting right is most of this decision. Three candidates were measured
(`tools/bench_papr.py`).

## What was measured

**1. Peak reduction is only worth what the binding constraint says it is.** Under an *EVM*
budget, iterative clipping is worthless — clipping is itself distortion, so it spends the
same budget the PA is spending, and at a fixed EVM target the clipped waveform delivers
slightly *less* average power than the plain one. Under a *splatter* budget, which is the
constraint that actually applies on a shared HF band, clip-and-filter wins: the filter half
puts the out-of-band regrowth back where it belongs, so the transmitter can be driven harder
without splashing into the neighbouring QSO.

Delivered receiver SNR, PA saturation fixed, best drive chosen per variant:

| Clip target | PAPR | EVM | Soft PA (p=2), splatter ≤ −40 dB | ALC-like PA (p=5), ≤ −40 dB |
|---|---|---|---|---|
| none | 10.6 dB | — | reference | reference |
| 7 dB | 7.6 dB | −31.9 dB | +0.25 dB | +0.50 dB |
| 6 dB | 6.7 dB | −26.9 dB | +0.50 dB | +0.99 dB |
| 5 dB | 5.9 dB | −22.8 dB | +0.98 dB | +1.71 dB |
| 4 dB | 5.1 dB | −19.5 dB | +1.46 dB | +2.20 dB |

The gain is the same at a −20 dB and a −30 dB noise floor, confirming it is genuine link
gain and not a distortion trade. It is roughly twice as large for the harder, more
ALC-like amplifier — which is the one most amateur stations are actually running.

**2. The EVM ceiling bites the amplitude-modulated constellations, not simply the fastest
modes.** The first test of this used comfortable operating points and suggested only 64-QAM
cared. Re-run at each mode's *actual* threshold, where the link really sits, the split falls
between PSK and QAM — the clipper works hardest on exactly the outer constellation points
that QAM uses to carry information:

| Mode, at its threshold | none | t=7 | t=6 | t=5 |
|---|---|---|---|---|
| 8-PSK 1/2 @ +4.4 dB | 0.00 | 0.00 | 0.00 | 0.00 |
| 16-QAM 3/4 @ +9.9 dB | 0.00 | 0.00 | 0.05 | **0.45** |
| 64-QAM 5/6 @ +18 dB | 0.00 | 0.00 | 0.00 | **0.30** |

(frame error rate). 8-PSK rides through the hardest clipping tested; 16-QAM 3/4 goes from
zero to 45 % frame errors at the 5 dB target.

**3. Tone reservation does not pay for itself.** Reserving 2–8 of the 42 data carriers to
carry a peak-cancelling signal buys 0.1–0.8 dB of PAPR for 5–19 % of the payload. Against
clip-and-filter's 3–5 dB for no payload at all, it is not close.

**4. DFT-spreading was not pursued.** It would reduce PAPR substantially, but it is a
different air interface: it breaks the per-carrier comb-pilot channel estimation that
ADR-0002 is built on and that the fading results depend on. Re-opening the numerology to
chase a benefit clip-and-filter already captures, at zero cost to the air interface, is not
a trade worth making in v1.

## Decision

Adopt **iterative clip-and-filter in the transmitter**, with a mode-dependent target:

* **Constant-modulus modes (BPSK, QPSK, 8-PSK): clip to 5 dB.** They carry no information in
  amplitude, so −22.8 dB EVM costs them almost nothing, and they are the modes a weak link
  actually runs on — which is where margin is worth having. Delivered gain ≈ **+1.0 dB**
  (soft PA) to **+1.7 dB** (ALC-like PA).
* **Amplitude-modulated modes (16-QAM, 64-QAM): clip to 7 dB.** −31.9 dB EVM keeps their
  outer points intact. Delivered gain ≈ **+0.25 … +0.50 dB**.

Measured at every threshold in the sweep, this costs at most ≈ 0.2 dB of receiver
sensitivity (QPSK 1/2 is the worst case) in exchange for 1.0–1.7 dB of delivered power — a
net gain of roughly a dB on the modes that matter, and no regression anywhere.

Four iterations. The filter is the *same* linear-phase FIR the passband chain already
applies, used causally with its constant group delay removed, so a streaming transmitter
implements this exactly as the model does and it can emit nothing the transmitter would not.
Average power is renormalised after clipping, so the peak reduction is a true peak reduction
and not a level change.

**Reject** tone reservation and DFT-spreading, on the numbers above.

## Consequences

* The receiver is unchanged and needs no knowledge of any of this — a peak-reduced frame is
  an ordinary frame with a little extra in-band noise.
* **The transmitted waveform changed, so the golden vectors were regenerated.** This is the
  deliberate air-interface-adjacent change that `test_vectors.py` requires an ADR for.
* PAPR of a transmitted burst is now 5.7 dB (BPSK, QPSK, 8-PSK) and 7.3 dB (16-QAM, 64-QAM),
  from ≈ 10 dB.
  For `COMMUNITY-CONCERNS.md` #8 that is a > 4 dB reduction in the peak the ALC reacts to.
  The average power step between preamble and data widens slightly, from ≈ 0.6 dB to ≈ 1.1 dB,
  because the filter spreads clipped energy across symbol boundaries; ADR-0002's requirement
  was about level steps large enough to pump the ALC, and 1.1 dB is not one.
* `FrameTransmitter(papr_reduction=False)` restores the raw envelope, for comparisons and for
  a transmitter that is already linear enough not to care.
* The 64-QAM EVM floor is now the modem's own noise floor in a noiseless test: carrier SNR
  reads ≈ 32 dB rather than > 40 dB. That is ADR-0004 working as intended, not a regression.

## Follow-ups

* Re-measure on a real radio once Phase 3 can key one: the gain depends on how hard the
  rig's ALC is, and `p = 5` is a model, not a measurement.
* Separately noticed while measuring this: the interpolated (unmeasured) AWGN thresholds in
  `link/rate.py` for modes 9 and 12 are optimistic — both show high frame error at the values
  in the table, clipped or not. A sweep covering every mode, not just the seven measured
  ones, would fix the rate controller's table.
* The PA model is memoryless. Real amplifiers have memory, and a measured AM/AM–AM/PM curve
  would sharpen the drive recommendation the setup wizard gives the operator (§8).
