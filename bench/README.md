# Benchmarks

`baselines/` holds committed results that every DSP change is compared against. The release
pipeline runs `tools/bench_gate.py` — the seeded AWGN sweep in `gate_awgn.csv`, four modes,
thirty frames a point — and refuses to publish if any mode's 10 % frame-error point has
moved up by more than 0.3 dB. Regenerate that baseline (`--regenerate`) only after a
deliberate change to the waveform, and say why in the commit.

| File | Produced by | What |
|---|---|---|
| `ldpc_bg2_awgn.csv` | `python tools/bench_ldpc.py --max-blocks 1024 --target-errors 60` | BLER/BER vs E_s/N_0, TS 38.212 BG2, K′ = 480, BPSK, AWGN, RV0 rate matching |
| `phy_fer_phase1_uw.csv` | `python tools/bench_phy.py --frames 30` at commit `377be74` | FER / throughput vs SNR (3 kHz) per mode and ITU channel, **Phase 1 air interface** (3-symbol preamble with unique word, S&C-nominated detector) |
| `phy_fer.csv` | `python tools/bench_phy.py --frames 30` | same, current air interface (P2-3: PN type preamble, chip-signalled mode, PMF-FFT bank). Generated before ADR-0004 peak reduction; re-checked at all seven measured thresholds afterwards, worst shift ≈ 0.2 dB (QPSK 1/2), so the table still stands |
| `link_throughput.csv` | `python tools/bench_link.py --bytes 16000 --trials 3` | end-to-end link goodput vs SNR per channel: a whole session (connect, 16 kB, disconnect) with adaptive rate |
| `link_ramp.csv` | `python tools/bench_link.py --ramp --channels awgn,poor --snr 8,14 --bytes 24000 --trials 2` | the same under a ±8 dB triangular fade, 60 s period — rate-control tracking |
| `phy_fer_awgn14.csv` | `python tools/bench_phy.py --channels awgn --modes 0,1,...,13 --frames 30` | AWGN FER for **every** mode — the source of the rate controller's threshold table (`tools/update_rate_table.py`) |
| `gate_awgn.csv` | `python tools/bench_gate.py --regenerate` (AWGN, modes 0, 4, 8, 13, 30 frames) | the release gate's baseline, measured with the current air interface including ADR-0004 peak reduction and the P2-5 blanker. Against `phy_fer_awgn14.csv` (measured before both) modes 0, 4 and 13 are within 0.01 dB and QAM16-1/2 read 0.4 dB worse. A re-sweep of modes 6–10 settled it: 6, 7, 9 and 10 are unchanged to 0.02 dB, and mode 8 differs by two frames out of thirty at 6 dB (25 decode now, 27 before) — the 10 % crossing of the old sweep sat exactly on that grid point, so two frames move the interpolated threshold by 0.4 dB. Measurement resolution, not a change in the waveform: the other 16-QAM modes share the 7 dB clipping target and did not move |
| `impulsive.csv` | `python tools/bench_impulsive.py --frames 10 --modes 0,4,10` | FER vs impulsive-noise rate, with each P2-5 defence on and off |
| `chanest.csv` | `python tools/bench_chanest.py --frames 16 --snr-offsets 0,2` | linear vs Wiener channel estimation (the P2-6 decision) |
| `papr.csv` | `python tools/bench_papr.py` | PAPR / EVM / splatter per reduction technique, delivered SNR through a saturating PA, and end-to-end decoding (ADR-0004) |
| `link_throughput_phy.csv` | `python tools/bench_link.py --backend phy --channels awgn --snr 8,14 --bytes 4000 --trials 1` | two AWGN points re-run through the real modem, to validate the fast backend |

Conventions: E_s/N_0 per transmitted BPSK symbol; E_b/N_0 = E_s/N_0 − 10·log10(R).
All SNRs elsewhere in the project are referenced to a 3 kHz noise bandwidth (see
`docs/ROADMAP.md` §7).

Reference points from `ldpc_bg2_awgn.csv` (BLER = 10 %):

| Rate | E_b/N_0 (dB) | Shannon limit (dB) |
|---|---|---|
| 1/5 | ≈ 0.9 | −0.5 |
| 1/3 | ≈ 1.1 | −0.5 |
| 1/2 | ≈ 1.5 | 0.2 |
| 2/3 | ≈ 2.1 | 1.1 |
| 3/4 | ≈ 2.7 | 1.6 |
| 5/6 | ≈ 3.5 | 2.4 |

## PHY, current air interface (`phy_fer.csv`, P2-3, 30 frames/point, random ±100 Hz CFO and ±50 ppm SRO)

Minimum usable SNR (3 kHz noise bandwidth) for FER ≤ 10 %, linearly interpolated:

| Mode | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| BPSK 1/5 | −5.2 | +2.0 | +1.2 | −0.3 |
| BPSK 1/2 | −2.0 | +5.5 | +4.8 | +2.3 |
| QPSK 1/2 | +1.0 | +8.7 | +8.5 | +6.0 |
| 8-PSK 1/2 | +4.4 | +11.7 | +12.3 | +9.7 |
| 16-QAM 1/2 | +6.0 | +13.7 | +12.7 | +12.5 |
| 16-QAM 3/4 | +9.9 | +17.7 | +19.8 | > +24 |
| 64-QAM 5/6 | +16.9 | +24.8 | > +30 | > +31 |

Peak payload throughput at the top of each mode's usable range: 197 bps (BPSK 1/5) rising
to 5 556 bps (64-QAM 5/6) on AWGN; the fading columns match until the channel runs out of
SNR for the high modes (64-QAM 5/6 collapses to 185 bps on Poor). The matched-filter-bank
detector (P2-3) is what moves BPSK 1/5 on AWGN from −3.3 dB (acquisition-limited, Phase 1)
to −5.2 dB — the LDPC code, not acquisition, is now the floor. The high modes and the
fading columns are essentially unchanged from Phase 1, as expected: only the low-SNR
acquisition path changed.

## PHY, Phase 1 air interface (`phy_fer_phase1_uw.csv`, 30 frames/point, random ±100 Hz CFO and ±50 ppm SRO)

Minimum usable SNR (3 kHz noise bandwidth) for FER ≤ 10 %, linearly interpolated:

| Mode | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| BPSK 1/5 | −3.3 | +3.3 | +2.7 | +3.0 |
| BPSK 1/2 | −1.3 | +5.7 | +5.7 | +3.3 |
| QPSK 1/2 | +1.0 | +8.7 | +9.0 | +6.5 |
| 8-PSK 1/2 | +4.4 | +12.0 | +12.5 | +10.0 |
| 16-QAM 1/2 | +6.2 | +14.0 | +13.5 | +12.3 |
| 16-QAM 3/4 | +9.9 | +17.8 | +19.8 | > +24 |
| 64-QAM 5/6 | +16.9 | +24.8 | > +30 | > +31 |

Reading the fading columns: ITU Good/Moderate (0.1–0.5 Hz Doppler) hold one channel
realisation for the whole 1 s frame, so only the 2.3 kHz of frequency diversity helps and
the curves are shallow; Poor (1 Hz) changes within the frame and the interleaver turns
that into time diversity — hence Poor ≤ Moderate for the low modes. BPSK 1/5 on AWGN was
acquisition-limited in this sweep (the detector, not the code, stopped at −3 dB); the
P2-3 detector removes that limit (acquisition 100 % at −5 dB, 87 % at −7 dB).

## Link layer, goodput vs SNR (`link_throughput.csv`, P2-2)

A complete session — connect, 16 kB transfer, orderly disconnect — with the rate controller
free to pick any mode. Median of 3 trials; brackets give the fastest mode reached and the
efficiency against that channel's own capacity. "—" means the transfer did not finish inside
the time cap (the channel is below the most robust mode's threshold).

| SNR (3 kHz) | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| −4 | 102 (m0, 0.52) | — | — | — |
| +0 | 169 (m1, 0.23) | — | — | — |
| +4 | 578 (m4, 0.40) | 126 (m1, 0.36) | 140 (m1, 0.40) | 151 (m2, 0.28) |
| +8 | 1162 (m6, 0.39) | 299 (m4, 0.41) | 346 (m4, 0.48) | 595 (m4, 0.41) |
| +12 | 1696 (m9, 0.51) | 859 (m6, 0.52) | 819 (m6, 0.56) | 1034 (m6, 0.63) |
| +16 | 2003 (m11, 0.40) | 1246 (m9, 0.42) | 1419 (m9, 0.48) | 1565 (m11, 0.47) |
| +20 | 2106 (m13, 0.38) | 1696 (m11, 0.51) | 1565 (m11, 0.47) | 1745 (m13, 0.35) |

**Efficiency.** Against the raw payload rate of the mode in use, goodput is ≈ 0.71–0.75 once
a transfer is long enough to amortise the rate ramp (250 kB on AWGN: 0.71 at +12 dB, 0.75 at
+20 dB). A 16 kB session reaches ≈ 0.5, the rest going to the climb from the most robust mode.
What remains is structural, not waste:

* **per-burst turnaround** — the IRS waits for silence to know a burst ended, then its ACK
  occupies 0.43 s. With the start-of-frame signal (below) that wait is 0.12 s rather than a
  whole 1.05 s data frame.
* **rate ramp** — the controller starts on the most robust mode and climbs at most two modes
  per burst, so short transfers spend much of their life below the mode they finish on.
* **deliberate conservatism** — the hysteresis band keeps the link a little below the edge.

Burst length was swept (4/6/8/12/16 frames): goodput peaks at **6–8** and *falls* beyond that,
because the ACK is also the rate-control feedback — longer bursts mean fewer decisions per
transfer and a slower climb, costing more than the saved turnarounds. The default is 6.

### P2-2a — start-of-frame signal

A PHY that reports a detected preamble (`PhyTiming.preamble_detect_s`, four symbol periods
= 0.124 s for this waveform) lets the receiver hold its ACK as soon as it hears the next
frame begin, instead of assuming that a frame-time of quiet means the burst is over. The
burst length cannot simply be put in the header: the header has to be identical across the
retransmissions the receiver soft-combines.

| | +4 dB | +12 dB | +20 dB | 250 kB @ +12 | 250 kB @ +20 |
|---|---|---|---|---|---|
| without | 503 | 1510 | 1877 | 2103 (0.63) | 3709 (0.67) |
| with | 567 | 1696 | 2106 | 2369 (0.71) | 4177 (0.75) |

≈ +13 % throughput across the range.

### P2-2b — the margin learns the channel

A failed burst is a measurement: mode *m* dying at SNR *s* says this channel needs more than
`s − threshold[m]` dB of margin. The controller now jumps toward that figure (capped at 3 dB
per burst) instead of creeping up in fixed 1.5 dB steps, and holds what it learned — the
margin decays once every three clean bursts rather than every one. Before the first failure
there is nothing to protect, so decay is immediate and a good link still converges fast.

The effect is largest exactly where the AWGN table is most wrong. Against the previous
controller (16 kB sessions): Moderate +16 dB 1096 → 1419 bps (+29 %), Good +20 dB 1378 →
1696 (+23 %), AWGN +20 dB 1877 → 2106 (+12 %). Note that at +12 dB on the fading channels it
now settles *lower* — mode 6 rather than mode 8 — and still delivers more: the old behaviour
was overshooting onto a mode the channel could not hold and paying for it in retransmissions.

## Link layer, rate tracking under a fade (`link_ramp.csv`)

A ±8 dB triangular fade (60 s period) around the stated mean, 24 kB transfers. Every run
completed. On AWGN at a +8 dB mean the controller made 16–20 mode changes and 15–18
retransmissions; on Poor at the same mean, 33–34 changes and 60–70 retransmissions. Nothing
stalled and no session was lost, which is the property under test — the hysteresis stops the
controller flapping on a static channel without stopping it tracking a moving one.

## Backend cross-check (`link_throughput_phy.csv`)

The fast backend models only the *error process*; all protocol timing is shared with the
real-PHY harness. Re-running two AWGN points through the real modem reproduces the
lossy-pipe goodput exactly — 764.7 bps at +8 dB and 849.8 bps at +14 dB on both — because at
those SNRs no frame fails in either backend, so only the timing matters.

## PAPR (`papr.csv`, P2-4 / ADR-0004)

Raw OFDM measures 9–10 dB PAPR. Because an SSB transmitter is driven at a fixed peak,
that is link margin thrown away — but only the constraint that actually binds decides how
much is recoverable. Under an *EVM* budget, clipping is worthless (it spends the same budget
the PA does). Under a *splatter* budget, which is what applies on a shared band, clip-and-
filter lets the transmitter be driven harder without splashing:

| Clip target | PAPR | EVM | splatter | gain, soft PA (p=2) | gain, ALC-like PA (p=5) |
|---|---|---|---|---|---|
| none | 10.6 dB | — | −67 dB | reference | reference |
| 7 dB | 7.6 dB | −31.9 dB | −70 dB | +0.25 dB | +0.50 dB |
| 6 dB | 6.7 dB | −26.9 dB | −70 dB | +0.50 dB | +0.99 dB |
| 5 dB | 5.9 dB | −22.8 dB | −70 dB | +0.98 dB | +1.71 dB |
| 4 dB | 5.1 dB | −19.5 dB | −69 dB | +1.46 dB | +2.20 dB |

(Delivered receiver SNR at a −40 dB splatter limit, best drive chosen per variant. The gain
is the same at a −20 dB and a −30 dB noise floor, so it is genuine link gain.)

The EVM ceiling bites the amplitude-modulated constellations, and it has to be measured at
each mode's *threshold* to show up: at comfortable SNRs clipping to 5 dB looks free, but at
its actual threshold 16-QAM 3/4 goes from 0 % to 45 % frame errors, while 8-PSK 1/2 rides
through the hardest clipping tested. Hence ADR-0004's split — 5 dB for BPSK/QPSK/8-PSK,
7 dB for 16-QAM/64-QAM. Across every measured threshold that costs ≤ 0.2 dB of receiver
sensitivity for 1.0–1.7 dB of delivered power. Tone reservation was measured and rejected:
0.1–0.8 dB of PAPR for 5–19 % of the payload.

## Rate-controller thresholds (`phy_fer_awgn14.csv`, P2-2b follow-up)

`link/rate.py` picks modes from a table of minimum usable SNR. Seven of the fourteen entries
used to be interpolated guesses, and the P2-4 work caught two of them costing frames. This
sweep covers every mode, and `tools/update_rate_table.py --apply` regenerates the table from
it. The guesses were optimistic exactly where it hurts most — the fast modes:

| Mode | 9 (16-QAM 2/3) | 11 (64-QAM 2/3) | 12 (64-QAM 3/4) | 7 (8-PSK 2/3) |
|---|---|---|---|---|
| guessed | +8.0 | +12.5 | +14.5 | +7.5 |
| measured | +8.9 | +13.9 | +15.6 | +6.9 |

The seven previously-measured modes moved by 0.0–0.4 dB, which is also the clearest
measurement of what ADR-0004 peak reduction cost in sensitivity: ≤ 0.4 dB, against the
1.0–1.7 dB of transmit power it bought.

## Impulsive noise (`impulsive.csv`, P2-5)

Frame error rate against the fraction of samples hit by a burst 25 dB above the noise, each
defence on and off. Impulsive noise is the impairment OFDM handles *worse* than a
single-carrier waveform — the FFT spreads one hot sample across all 57 carriers of its symbol.

| P(impulse) | none | per-symbol σ² | blanker | both | blanked |
|---|---|---|---|---|---|
| 0 | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 % |
| 0.002 | 0.10 | 0.00 | 0.00 | 0.00 | 0.19 % |
| 0.005 | 0.60 | 0.50 | 0.00 | 0.00 | 0.49 % |
| 0.01 | 1.00 | 1.00 | 0.00 | 0.00 | 0.96 % |
| 0.05 | 1.00 | 1.00 | 0.00 | 0.00 | 4.77 % |
| 0.10 | 1.00 | 1.00 | 0.00 | 0.00 | 9.28 % |
| 0.20 | 1.00 | 1.00 | 0.30 | 0.20 | 18.28 % |

(QPSK 1/2 at +4 dB; BPSK 1/5 and 16-QAM 3/4 behave the same way.) Three things to read off it:

* **The blanker does nearly all the work.** Without it the link is a total loss from 1 % of
  samples upward; with it, nothing is lost until 20 %.
* **Per-symbol noise variance is a second-order help**, worth something only at low impulse
  rates. That is the erasure mechanism working as designed and also its limit: it can
  discount the damaged symbols only while most symbols are clean.
* **The blanker is free when there is nothing to blank.** At P = 0 it removes 0.00 % of
  samples and changes no decode, at any mode or SNR — which is why it is on by default. The
  blanked fraction tracks the impulse rate almost exactly (18.28 % at P = 0.20), so it is
  finding the impulses and essentially nothing else.

Its limit is a *sustained* burst: the reference level is a median of segment medians, which
holds only while the burst is a minority of the window it looks at
(`NoiseBlanker.robust_span_samples`, ≈ 57 ms at the defaults).

## Channel estimation: linear vs Wiener (`chanest.csv`, P2-6 — **not adopted**)

P2-6 was conditional: improved channel estimation *if benchmarks justify it*. They did not,
and the default estimator stays linear-in-frequency with a 3-tap time average. The work and
the measurements are kept because the reason is specific and reopenable, not a dead end.

**The potential is real.** Measured against the true channel — a noiseless reference run of
the same fading realisation — Wiener interpolation matched to the delay spread actually
present beats linear interpolation of the same pilots:

| Channel | linear | Wiener, best matched τ |
|---|---|---|
| AWGN @ +4 dB | −7.5 dB | **−12.8** (τ = 0.5 ms) |
| ITU Good @ +9 dB | −7.4 | **−11.8** (0.5 ms) |
| ITU Moderate @ +10 dB | −10.1 | **−12.3** (1.0 ms) |
| ITU Poor @ +8 dB | −5.4 | **−6.7** (2.0 ms) |

(normalised interpolation error; lower is better.) The match matters enormously: the same
filter designed for 3 ms instead of 1 ms on Moderate gives −8.8 dB, worse than linear.

**It does not survive into frame error rate.** End to end the two are identical on AWGN and
Wiener is consistently worse on every fading channel — by 0.12 to 0.62 in FER:

| | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| Wiener vs linear, FER delta | 0.00 | −0.12 … −0.62 | −0.19 … −0.50 | −0.44 … |

Three things were tried and measured before concluding this:

1. **A single robust worst-case design** (the textbook recommendation) — loses nearly
   everywhere, because most channels are much flatter than the worst case.
2. **Estimating the delay spread** from the pilot impulse response — the 15-point transform
   leaks badly enough to overestimate the spread two- to four-fold on fading channels.
3. **Selecting the design by leave-one-pilot-out cross-validation**, which sidesteps leakage
   by scoring each candidate on held-out prediction error. This picks sensible designs and
   still does not recover the gain.

The leading explanation is that the two smoothers overlap: by the time the frequency filter
runs, the ±1-symbol time average has already removed most of the pilot noise, so the Wiener
filter's noise-averaging buys little while its design mismatch still costs. If P2-6 is
reopened it should be as a *joint* 2-D design, not two separable filters applied in sequence.
