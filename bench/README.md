# Benchmarks

`baselines/` holds committed results that every DSP change is compared against (CI will
fail a PR that regresses a curve by more than 0.3 dB once `tools/bench` exists in full).

| File | Produced by | What |
|---|---|---|
| `ldpc_bg2_awgn.csv` | `python tools/bench_ldpc.py --max-blocks 1024 --target-errors 60` | BLER/BER vs E_s/N_0, TS 38.212 BG2, K′ = 480, BPSK, AWGN, RV0 rate matching |
| `phy_fer_phase1_uw.csv` | `python tools/bench_phy.py --frames 30` at commit `377be74` | FER / throughput vs SNR (3 kHz) per mode and ITU channel, **Phase 1 air interface** (3-symbol preamble with unique word, S&C-nominated detector) |
| `phy_fer.csv` | `python tools/bench_phy.py --frames 30` | same, current air interface (P2-3: PN type preamble, chip-signalled mode, PMF-FFT bank) |
| `link_throughput.csv` | `python tools/bench_link.py --bytes 16000 --trials 3` | end-to-end link goodput vs SNR per channel: a whole session (connect, 16 kB, disconnect) with adaptive rate |
| `link_ramp.csv` | `python tools/bench_link.py --ramp --channels awgn,poor --snr 8,14 --bytes 24000 --trials 2` | the same under a ±8 dB triangular fade, 60 s period — rate-control tracking |
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
free to pick any mode. Median of 3 trials; the mode in brackets is the fastest one reached.
"—" means the transfer did not finish inside the time cap (the channel is below the most
robust mode's threshold).

| SNR (3 kHz) | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| −4 | 91 (m0) | — | — | — |
| +0 | 203 (m1) | — | — | — |
| +4 | 578 (m4) | 112 (m2) | 124 (m2) | 173 (m2) |
| +8 | 1032 (m6) | 333 (m4) | 385 (m4) | 568 (m5) |
| +12 | 1510 (m9) | 764 (m8) | 778 (m8) | 924 (m8) |
| +16 | 1771 (m11) | 1179 (m10) | 1096 (m10) | 1391 (m11) |
| +20 | 1877 (m13) | 1378 (m12) | 1394 (m11) | 1548 (m13) |

**Efficiency.** Goodput is 0.55–0.67 of the raw payload rate of the mode in use once a
transfer is long enough to amortise the rate ramp (16 kB reaches ≈ 0.55, 250 kB ≈ 0.67).
The gap is structural, not waste in the code, and splits three ways:

* **≈ 25 % fixed cost per burst** — the IRS waits one whole data-frame time of silence to be
  sure the burst has ended (it cannot be told: the burst length would have to sit in the
  header, and the header must not change between the identical retransmissions the receiver
  soft-combines), then an ACK occupies 0.43 s. A start-of-frame (preamble-detected) signal
  from the PHY to the link layer would cut that wait from 1.5 s to ≈ 0.45 s and is the single
  biggest throughput win available — recorded as a P2-2 follow-up in the roadmap.
* **rate ramp** — the controller starts on the most robust mode and climbs at most two modes
  per burst, so short transfers spend much of their life below the mode they end on.
* **deliberate conservatism** — the hysteresis band (margin + 1.5 dB) keeps the link one mode
  below the edge. At +12 dB on AWGN it settles on mode 9 rather than mode 10.

Burst length was swept (4/6/8/12/16 frames): goodput peaks at **6–8 frames** and *falls*
beyond that, because the ACK is also the rate-control feedback — longer bursts mean fewer
decisions per transfer and a slower climb, which costs more than the saved turnarounds.
The default is 6.

**Fading columns.** The controller only knows the AWGN threshold table and widens its margin
when frames fail, so on Good/Moderate it ends one or two modes below what that channel could
carry — the price of not having per-channel thresholds. Learning a per-channel offset is the
obvious next refinement.

## Link layer, rate tracking under a fade (`link_ramp.csv`)

A ±8 dB triangular fade (60 s period) around the stated mean, 24 kB transfers. Every run
completed. On AWGN at a +8 dB mean the controller made 20–24 mode changes and 22–23
retransmissions; on Poor at the same mean, 41–45 changes and 73–78 retransmissions. Nothing
stalled and no session was lost, which is the property being tested — the hysteresis stops
the controller from flapping on a static channel without stopping it from tracking a moving
one.

## Backend cross-check (`link_throughput_phy.csv`)

The fast backend models only the *error process*; all protocol timing is shared with the
real-PHY harness. Re-running two AWGN points through the real modem reproduces the
lossy-pipe goodput exactly — 674.7 bps at +8 dB and 756.4 bps at +14 dB on both — because at
those SNRs no frame fails in either backend, so only the timing matters.
