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
| `link_ramp.csv` | `python tools/bench_link.py --ramp --channels awgn,poor --snr 8,14 --bytes 24000 --trials 2` | the same under a ±8 dB triangular fade, 60 s period — rate-control tracking. `--bandwidth 500` runs the narrow table; `--rate key=value,…` overrides the controller's tunables, for one controller against another (ADR-0007) |
| `phy_fer_awgn14.csv` | `python tools/bench_phy.py --channels awgn --modes 0,1,...,13 --frames 30` | AWGN FER for **every** mode — the source of the rate controller's threshold table (`tools/update_rate_table.py`) |
| `phy_fer_500.csv` | `python tools/bench_phy.py --bandwidth 500 --frames 30` (2026-09-16) | the **500 Hz waveform** (P7-0, the floor family of ADR-0009): FER / throughput vs SNR (3 kHz) for all thirteen narrow modes on AWGN, Good, Moderate and Poor — the source of the narrow rate table (`tools/update_rate_table.py --bandwidth 500`) |
| `floor_500.csv` | `python tools/bench_floor.py --bandwidth 500` (2026-09-16) | **where the 500 Hz floor breaks** (ADR-0009): per frame — the three slowest modes on their own layouts and both control frames — how many of twenty were acquired within half a symbol, decoded through the detector, and decoded with genie timing, on AWGN, Good and Poor |
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

## PHY, the 500 Hz waveform (`phy_fer_500.csv`, P7-0 and ADR-0009, 30 frames/point, random ±100 Hz CFO and ±50 ppm SRO)

The narrow waveform (`docs/spec/air-interface.md` §2.3; ADR-0002's P7-0 amendment, the
floor family of ADR-0009) at the same 3 kHz-referenced SNR as the wide one — the same
transmitter power into the same noise, which is how an operator compares them. Minimum
usable SNR for FER ≤ 10 %, interpolated (rerun 2026-09-16 on the thirteen-mode table; the
fading-channel crossings of the floor modes sit on shallow curves and move a decibel or two
between runs of thirty frames):

| Narrow mode | AWGN | ITU Good | ITU Moderate | ITU Poor |
|---|---|---|---|---|
| 0 QPSK 1/10 · floor frame | −12.4 | −4.0 | −1.0 | −5.0 |
| 1 QPSK 1/5 · floor frame | −10.2 | −1.0 | −4.5 | −7.0 |
| 2 QPSK 1/3 | −6.0 | +1.0 | +2.0 | −1.3 |
| 3 QPSK 1/2 (control mode) | −5.2 | +4.0 | +3.5 | +0.0 |
| 4 QPSK 2/3 | −3.6 | +5.5 | +5.0 | +4.0 |
| 5 8-PSK 1/2 | −2.1 | +7.0 | +8.0 | +4.5 |
| 6 8-PSK 2/3 | +0.5 | +10.0 | > +11 | > +12 |
| 7 16-QAM 1/2 | −0.1 | +9.0 | +9.5 | +8.0 |
| 8 16-QAM 2/3 | +2.0 | +12.0 | +14.0 | > +15 |
| 9 16-QAM 3/4 | +3.5 | +14.0 | > +15 | > +16 |
| 10 64-QAM 2/3 | +6.9 | +17.0 | > +19 | > +20 |
| 11 64-QAM 3/4 | +8.8 | +18.0 | > +21 | > +22 |
| 12 64-QAM 5/6 | +10.4 | +21.0 | > +22 | > +23 |

What the table says. **The floor family (ADR-0009) moves the narrow floor from −5.2 to
−12.4 dB on AWGN** — QPSK 1/10 on a 4.2 s frame behind an eight-symbol preamble, 19 bytes a
frame, 36 bit/s of frame air time — with QPSK ⅕ (41 bytes, 78 bit/s) at −10.2 and QPSK ⅓ on
the ordinary frame at −6.0 as the rungs up to the control mode. On the fading channels the
floor frames buy 8–9 dB over the old floor on Good and 5–7 dB on Poor: a 4.2 s frame spans
several fades on Poor, which the interleaver turns into diversity, while on Good (0.1 Hz
Doppler) a frame still sits inside one fade and the shallow curves show it. **Modes 3–12
are the P7-0 table**, unchanged within a tenth of a decibel on AWGN: QPSK ½ on twelve
carriers decodes at −5.2 dB, the wide table's BPSK ⅕ at −5.2 — the ≈ 6.8 dB a 500 Hz
signal gains per carrier pays for the four rate steps — and the AWGN thresholds sit
6.4–7.1 dB below the wide table's for the same (modulation, rate). **On fading channels the
narrow waveform gives some of that back**: twelve carriers over 480 Hz have a fifth of the
frequency diversity, so a fade takes more of the frame with it, and the top modes never
reach 10 % FER on Poor within the sweep, where 16-QAM ¾ at 2 300 Hz does not either. P9-5
(time diversity) is the answer to that. Best single mode: 139 bit/s at −12 dB and 311 at
−10 on AWGN (the floor modes), 114 bit/s at −10 dB on Good and 111 on Poor; 1 040 bit/s
(64-QAM ⅚) on AWGN from +10.4 dB; 901 bit/s on Good and 654 on Moderate at +20 dB, and
359 bit/s on Poor at +12.

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
| +0 | 208 (m1, 0.29) | — | — | — |
| +4 | 544 (m3, 0.37) | 126 (m2, 0.64) | 140 (m2, 0.71) | 191 (m3, 0.36) |
| +8 | 1246 (m6, 0.56) | 324 (m3, 0.45) | 383 (m3, 0.53) | 601 (m5, 0.41) |
| +12 | 1879 (m9, 0.56) | 910 (m8, 0.55) | 910 (m8, 0.62) | 1140 (m8, 0.69) |
| +16 | 2495 (m10, 0.50) | 1471 (m10, 0.67) | 1403 (m10, 0.64) | 1770 (m10, 0.53) |
| +20 | 3399 (m12, 0.61) | 2222 (m11, 0.67) | 2144 (m11, 0.65) | 2547 (m11, 0.57) |

Regenerated 2026-09-16 with the faster climb (ADR-0007) and the faster start (ADR-0008):
a learned margin is given back at an accelerating rate once clean burst follows clean
burst, and a session starts where the connect frames measured it instead of climbing
from the most robust mode — which is most of the gain in these 16 kB figures at +8 dB
and above. Against the previous controller
on the same bench, +1.8 % net over this grid with no point worse than 0 % (+22 % on Good
and Moderate at +8 dB, +18 % on Poor), +3.2 % on the narrow table and +5.2 % under the
fade; the one loss anywhere is −3.4 % on Poor at +4 dB on the narrow table, where the
controller probes the next mode more often. The transfer that found it — 16 kB at 12 dB
over 500 Hz with one burst lost to a collision — goes from never recovering the top mode
to recovering it eight bursts later.

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

## Link layer, the start of a session (P9-2, ADR-0008)

`python tools/bench_link.py --bytes 2000 --snr 4,8,12,16,20 --trials 3` (and `--bandwidth
500 --snr 4,8,12,16`): a 2 kB session — connect, transfer, orderly disconnect — is
dominated by the climb from the most robust mode. Since 2026-09-16 the acceptance carries
the SNR the request arrived at and the first burst starts two steps below what that
supports. Median seconds per session, before → after:

| SNR (3 kHz) | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| +4 | 37.7 → 32.4 | 131.9 → 146.0 | 115.4 → 125.5 | 91.2 → 75.4 |
| +8 | 30.3 → 18.7 | 42.9 → 39.2 | 42.9 → 39.7 | 35.5 → 25.6 |
| +12 | 29.3 → 13.5 | 32.4 → 21.9 | 33.4 → 21.9 | 29.3 → 19.8 |
| +16 | 29.3 → 10.3 | 29.3 → 16.6 | 29.3 → 15.6 | 29.3 → 12.4 |
| +20 | 29.3 → 9.3 | 29.3 → 12.4 | 29.3 → 12.4 | 29.3 → 11.4 |

Wide table; the whole grid 887 → 673 s. On the narrow table 1033 → 940 s, no point worse
than +3.7 %. The +4 dB fading points are within the noise of three trials (ten trials on
Good and Poor at +8 dB: −7 % and −28 %). Through the real modem at 12 dB: 29.3 → 14.5 s
wide, 402 → 528 bit/s narrow.

## Link layer, rate tracking under a fade (`link_ramp.csv`)

A ±8 dB triangular fade (60 s period) around the stated mean, 24 kB transfers. Every run
completed. On AWGN at a +8 dB mean the controller made 20–21 mode changes and 25–27
retransmissions; on Poor at the same mean, 44–46 changes and 70–76 retransmissions. Nothing
stalled and no session was lost, which is the property under test — the hysteresis stops the
controller flapping on a static channel without stopping it tracking a moving one. The
faster climb (ADR-0007) tracks the fade more actively than its predecessor (a few more
changes and retransmissions per run) and carries 5 % more through it.

## Backend cross-check (`link_throughput_phy.csv`)

The fast backend models only the *error process*; all protocol timing is shared with the
real-PHY harness. Re-running two AWGN points through the real modem reproduces the
lossy-pipe goodput exactly at +8 dB — 927.6 bps on both — and within 5 % at +14 dB (1535
against 1617 bps): no frame fails in either backend, so only the timing matters, and the
one difference is that the modem's SNR estimate of the connect frame reads half a decibel
under the pipe's nominal figure, which starts the session one mode lower (5 against 6)
and ends the climb one lower (9 against 10). Where frames do
fail the two differ: the pipe's error process is a logistic 1.2 dB⁻¹ steep, while the
modem's measured FER curves are cliffs a decibel wide (`phy_fer_500.csv`, mode 8: 50 % at
8 dB, 0 % at 9 dB), so the pipe fails the odd frame a few dB above threshold that the modem
would not. Anything that turns on FER near threshold is settled on the `phy` backend.

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
