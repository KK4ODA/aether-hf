# Benchmarks

`baselines/` holds committed results that every DSP change is compared against (CI will
fail a PR that regresses a curve by more than 0.3 dB once `tools/bench` exists in full).

| File | Produced by | What |
|---|---|---|
| `ldpc_bg2_awgn.csv` | `python tools/bench_ldpc.py --max-blocks 1024 --target-errors 60` | BLER/BER vs E_s/N_0, TS 38.212 BG2, K′ = 480, BPSK, AWGN, RV0 rate matching |
| `phy_fer_phase1_uw.csv` | `python tools/bench_phy.py --frames 30` at commit `377be74` | FER / throughput vs SNR (3 kHz) per mode and ITU channel, **Phase 1 air interface** (3-symbol preamble with unique word, S&C-nominated detector) |
| `phy_fer.csv` | `python tools/bench_phy.py --frames 30` | same, current air interface (P2-3: PN type preamble, chip-signalled mode, PMF-FFT bank) |

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
