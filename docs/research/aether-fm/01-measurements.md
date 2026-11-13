# Measurements (scratch model)

These are estimates from a model written for the Aether FM proposal ([00-proposal.md](00-proposal.md)) on 2026-11-13, not committed benchmarks. The project's rule needs a committed curve behind any performance claim, and Phase 2 replaces these with one. The scripts that made them are in [`scratch/`](scratch/), frozen as they ran, with `sweep.json`, the sweep behind the tables.

## The model

- **Transmit, voice path:** a 300 Hz high-pass (3rd order), 750 µs pre-emphasis (zero at 212 Hz, pole at 6 kHz, unity at 1 kHz), a hard limiter at the peak deviation, a 4th-order 3 kHz splatter filter.
- **Transmit, flat port:** a DC block, no emphasis, no limiter; over-deviation shows as IF-filter distortion.
- **The air:** FM at ±5 kHz with a 6th-order IF at ±7.5 kHz (25 kHz channel), or ±2.5 kHz at ±4.25 kHz (12.5 kHz); the flat port at ±3 kHz per unit. Complex white noise at the CNR in the IF's noise bandwidth, and a phase-difference discriminator, so the threshold and its clicks arise by themselves.
- **Receive, voice path:** de-emphasis, a 6th-order 300 Hz high-pass, a 4th-order 3 kHz low-pass, and the radio's own floor 42 dB under a full-deviation tone. **Flat port:** a DC block and a 6 kHz low-pass.
- **The measure:** coherence between the modem's output and the received audio (Welch, 4 096-point segments, 6 s a point); SNDR(f) = C/(1−C); and the information rate, the integral of log2(1 + SNDR) over the signal's band with distortion counted as noise. That rate is a bound for Gaussian signalling, not a modem's rate.
- **Left out:** microphone ALCs and compressors (the TM-V71A has one), sound-card noise and clipping, emphasis mismatch between radios, real group delay, mobile fading, interference.

## Quality against drive

Effective SNDR (the flat SNDR that gives the same information rate over the signal's band) against drive: voice path, 25 kHz channel, `run_sweep.py`.

| Drive | CP-OFDM, CNR 18 dB | Single carrier, CNR 18 dB | CP-OFDM, CNR 9 dB | Single carrier, CNR 9 dB |
| --- | --- | --- | --- | --- |
| −24 dB | 9.5 dB | 10.7 dB | 1.1 dB | 3.0 dB |
| −22.5 dB | 11.1 dB | 12.1 dB | 2.8 dB | 4.2 dB |
| −21 dB | 12.5 dB | 13.5 dB | 4.1 dB | 5.6 dB |
| −19.5 dB | 14.1 dB | 14.9 dB | 5.5 dB | 6.9 dB |
| −18 dB | 15.5 dB | 16.4 dB | 6.7 dB | 8.3 dB |
| −16.5 dB | 17.1 dB | 17.8 dB | 8.7 dB | 9.7 dB |
| −15 dB | 18.5 dB | 19.1 dB | 10.1 dB | 11.3 dB |
| −13.5 dB | 19.9 dB | 20.5 dB | 11.7 dB | 12.5 dB |
| −12 dB | 21.3 dB | 21.8 dB | 13.0 dB | 13.7 dB |
| −10.5 dB | 22.5 dB | 23.1 dB | 14.5 dB | 15.2 dB |
| −9 dB | 23.1 dB | 24.4 dB | 15.8 dB | 16.6 dB |
| −7.5 dB | 22.2 dB | 25.4 dB | 16.6 dB | 17.9 dB |
| −6 dB | 19.8 dB | 25.1 dB | 16.1 dB | 18.6 dB |
| −4.5 dB | 17.1 dB | 22.3 dB | 14.9 dB | 18.2 dB |
| −3 dB | 14.7 dB | 19.0 dB | 13.4 dB | 16.7 dB |
| −1.5 dB | 12.8 dB | 16.6 dB | 12.0 dB | 15.1 dB |
| 0 dB | 11.3 dB | 14.8 dB | 10.7 dB | 13.8 dB |
| +1.5 dB | 10.2 dB | 13.3 dB | 9.7 dB | 12.6 dB |
| +3 dB | 9.3 dB | 12.2 dB | 8.9 dB | 11.7 dB |
| +4.5 dB | 8.6 dB | 11.3 dB | 8.3 dB | 10.9 dB |

The points are the sweep's own, 1.5 dB apart. The single carrier here is pre-compensated for the radio's pre-emphasis.

## Peak-to-average, real audio

The level over RMS exceeded by that share of samples, measured on 20 s of each signal at 48 kHz.

| Waveform | 10⁻³ | 10⁻⁴ |
| --- | --- | --- |
| Aether HF 2 300 Hz OFDM, a 16-QAM mode, ADR-0004's clip-and-filter | 9.82 dB | 10.30 dB |
| The same without peak reduction | 10.16 dB | 11.61 dB |
| CP-OFDM, 450–2 850 Hz, 40 Hz carriers, 16-QAM | 10.29 dB | 11.60 dB |
| DFT-spread OFDM, the same band, 16-QAM, no spectral shaping | 8.62 dB | 9.79 dB |
| Single carrier, 2 000 Bd, roll-off 0.2, QPSK | 6.96 dB | 7.69 dB |
| Single carrier, 2 000 Bd, roll-off 0.2, 16-QAM | 7.88 dB | 8.83 dB |
| Single carrier, 2 000 Bd, roll-off 0.2, 64-QAM | 8.17 dB | 9.14 dB |
| Single carrier, 2 000 Bd, roll-off 0.35, 16-QAM | 7.51 dB | 8.15 dB |
| Single carrier, 4 000 Bd, roll-off 0.2, 16-QAM (flat port) | 7.87 dB | 8.88 dB |
| 4-level continuous-phase FSK, 1 200 Bd | 3.01 dB | 3.01 dB |

## Best drive and drive window, every case

The information rate is the bound above, not a modem's rate. The window is the drive range that keeps 90 % of the best information rate, at the sweep's 1.5 dB resolution.

| Path | CNR | Waveform | Information rate at best drive | Best drive | 90 % window |
| --- | --- | --- | --- | --- | --- |
| Voice path, 25 kHz | 30 dB | CP-OFDM | 21.7 kb/s | −10.5 dB | 4.5 dB |
| Voice path, 25 kHz | 30 dB | Single carrier | 23.0 kb/s | −7.5 dB | 6.0 dB |
| Voice path, 25 kHz | 30 dB | Single carrier, pre-compensated | 23.5 kb/s | −7.5 dB | 6.0 dB |
| Voice path, 25 kHz | 30 dB | Aether HF 2 300 Hz | 20.9 kb/s | −10.5 dB | 4.5 dB |
| Voice path, 25 kHz | 18 dB | CP-OFDM | 18.5 kb/s | −9.0 dB | 4.5 dB |
| Voice path, 25 kHz | 18 dB | Single carrier | 19.6 kb/s | −7.5 dB | 4.5 dB |
| Voice path, 25 kHz | 18 dB | Single carrier, pre-compensated | 20.3 kb/s | −7.5 dB | 4.5 dB |
| Voice path, 25 kHz | 18 dB | Aether HF 2 300 Hz | 17.7 kb/s | −9.0 dB | 4.5 dB |
| Voice path, 25 kHz | 9 dB | CP-OFDM | 13.3 kb/s | −7.5 dB | 3.0 dB |
| Voice path, 25 kHz | 9 dB | Single carrier | 14.1 kb/s | −6.0 dB | 3.0 dB |
| Voice path, 25 kHz | 9 dB | Single carrier, pre-compensated | 14.9 kb/s | −6.0 dB | 3.0 dB |
| Voice path, 25 kHz | 9 dB | Aether HF 2 300 Hz | 12.5 kb/s | −7.5 dB | 6.0 dB |
| Voice path, 12.5 kHz | 30 dB | CP-OFDM | 19.7 kb/s | −9.0 dB | 6.0 dB |
| Voice path, 12.5 kHz | 30 dB | Single carrier, pre-compensated | 21.6 kb/s | −7.5 dB | 4.5 dB |
| Voice path, 12.5 kHz | 18 dB | CP-OFDM | 13.8 kb/s | −7.5 dB | 3.0 dB |
| Voice path, 12.5 kHz | 18 dB | Single carrier, pre-compensated | 15.8 kb/s | −6.0 dB | 3.0 dB |
| Voice path, 12.5 kHz | 9 dB | CP-OFDM | 8.2 kb/s | −4.5 dB | 6.0 dB |
| Voice path, 12.5 kHz | 9 dB | Single carrier, pre-compensated | 9.9 kb/s | −3.0 dB | 6.0 dB |
| Flat port, 25 kHz | 30 dB | CP-OFDM | 47.4 kb/s | −6.0 dB | 9.0 dB |
| Flat port, 25 kHz | 30 dB | Single carrier | 48.8 kb/s | −4.5 dB | 7.5 dB |
| Flat port, 25 kHz | 18 dB | CP-OFDM | 35.5 kb/s | −1.5 dB | 4.5 dB |
| Flat port, 25 kHz | 18 dB | Single carrier | 36.8 kb/s | +1.5 dB | 4.5 dB |
| Flat port, 25 kHz | 9 dB | CP-OFDM | 20.2 kb/s | −1.5 dB | 4.5 dB |
| Flat port, 25 kHz | 9 dB | Single carrier | 21.3 kb/s | 0.0 dB | 3.0 dB |

Drive is the RMS at the limiter relative to its clipping point (voice path), or the RMS deviation relative to ±3 kHz (flat port).

## The two-level probe

SNDR of 0.25 s of 16-QAM single carrier at the drive −1.5 dB and +1.5 dB, each segment its own transmission, data-aided least-squares fit; 40 trials a point, voice path, 25 kHz channel. "Right" is the share of trials whose difference points toward the optimum; at the optimum itself, the share within ±1.5 dB.

| CNR | Drive from the optimum | Mean ΔSNDR | Spread (SD) | Right |
| --- | --- | --- | --- | --- |
| 30 dB | −9 dB | +1.05 dB | 0.34 dB | 100 % |
| 30 dB | −6 dB | +0.63 dB | 0.43 dB | 90 % |
| 30 dB | −3 dB | +0.21 dB | 0.58 dB | 62 % |
| 30 dB | −1.5 dB | +0.26 dB | 0.51 dB | 68 % |
| 30 dB | 0 dB | −0.40 dB | 0.45 dB | 100 % |
| 30 dB | +1.5 dB | −2.43 dB | 0.54 dB | 100 % |
| 30 dB | +3 dB | −4.83 dB | 0.39 dB | 100 % |
| 30 dB | +6 dB | −4.20 dB | 0.47 dB | 100 % |
| 18 dB | −9 dB | +1.94 dB | 0.34 dB | 100 % |
| 18 dB | −6 dB | +1.48 dB | 0.38 dB | 100 % |
| 18 dB | −3 dB | +0.91 dB | 0.32 dB | 100 % |
| 18 dB | −1.5 dB | +0.70 dB | 0.41 dB | 93 % |
| 18 dB | 0 dB | +0.01 dB | 0.53 dB | 97 % |
| 18 dB | +1.5 dB | −1.95 dB | 0.53 dB | 100 % |
| 18 dB | +3 dB | −4.40 dB | 0.40 dB | 100 % |
| 18 dB | +6 dB | −4.22 dB | 0.39 dB | 100 % |
| 9 dB | −9 dB | +2.54 dB | 0.63 dB | 100 % |
| 9 dB | −6 dB | +2.36 dB | 1.13 dB | 100 % |
| 9 dB | −3 dB | +1.70 dB | 1.13 dB | 90 % |
| 9 dB | −1.5 dB | +1.40 dB | 0.76 dB | 93 % |
| 9 dB | 0 dB | −0.17 dB | 1.42 dB | 70 % |
| 9 dB | +1.5 dB | −2.65 dB | 0.84 dB | 100 % |
| 9 dB | +3 dB | −3.54 dB | 0.84 dB | 100 % |
| 9 dB | +6 dB | −3.09 dB | 0.48 dB | 100 % |

## A single transmission at one level

The negative result behind E.3: the residual of a 96-tap least-squares fit, split by the modem signal's envelope. Error power in the top tenth of the envelope over error power around its median rises only to 1.3–2.0 dB just past the optimum, then falls back in deep overdrive. Voice path, 25 kHz channel, single carrier pre-compensated; this SNDR comes from the time-domain fit, so it differs slightly from the first table's.

| CNR | Drive | SNDR | Error at peaks over error at the median |
| --- | --- | --- | --- |
| 30 dB | −15.0 dB | 23.8 dB | −0.07 dB |
| 30 dB | −10.5 dB | 25.6 dB | 0.12 dB |
| 30 dB | −7.5 dB | 26.0 dB | 0.28 dB |
| 30 dB | −6.0 dB | 24.9 dB | 1.52 dB |
| 30 dB | −4.5 dB | 21.2 dB | 2.01 dB |
| 30 dB | −1.5 dB | 14.5 dB | 1.11 dB |
| 18 dB | −15.0 dB | 19.7 dB | −0.01 dB |
| 18 dB | −10.5 dB | 23.0 dB | 0.12 dB |
| 18 dB | −7.5 dB | 24.3 dB | 0.12 dB |
| 18 dB | −6.0 dB | 23.9 dB | 1.33 dB |
| 18 dB | −4.5 dB | 20.8 dB | 1.96 dB |
| 18 dB | −1.5 dB | 14.4 dB | 1.08 dB |
| 9 dB | −15.0 dB | 11.9 dB | 0.22 dB |
| 9 dB | −10.5 dB | 16.1 dB | −0.06 dB |
| 9 dB | −7.5 dB | 18.7 dB | 0.36 dB |
| 9 dB | −6.0 dB | 19.6 dB | 0.22 dB |
| 9 dB | −4.5 dB | 18.3 dB | 0.73 dB |
| 9 dB | −1.5 dB | 13.6 dB | 1.07 dB |

## The linear equaliser's penalty

Per-frequency coding (OFDM's ideal) against a linear MMSE frequency-domain equaliser, from the measured SNDR(f) over the flat part of the single carrier's band (voice 850–2 450 Hz, flat port 1 100–4 300 Hz), at the best drive.

| Path | CNR | SNDR(f) across the band | Per-frequency coding | Linear MMSE | Gap |
| --- | --- | --- | --- | --- | --- |
| Voice, pre-compensated | 30 dB | 27.6 to 34.7 dB | 31.9 dB | 31.5 dB | 0.38 dB |
| Voice, pre-compensated | 18 dB | 22.6 to 31.1 dB | 27.3 dB | 26.7 dB | 0.59 dB |
| Voice, pre-compensated | 9 dB | 14.7 to 24.5 dB | 19.4 dB | 18.7 dB | 0.66 dB |
| Voice, not compensated | 30 dB | 29.3 to 32.3 dB | 31.1 dB | 31.1 dB | 0.04 dB |
| Voice, not compensated | 18 dB | 25.0 to 28.0 dB | 26.7 dB | 26.7 dB | 0.04 dB |
| Voice, not compensated | 9 dB | 17.2 to 20.1 dB | 18.7 dB | 18.7 dB | 0.03 dB |
| Flat port | 30 dB | 23.6 to 36.9 dB | 29.3 dB | 28.1 dB | 1.15 dB |
| Flat port | 18 dB | 18.4 to 31.4 dB | 23.9 dB | 22.8 dB | 1.13 dB |
| Flat port | 9 dB | 8.9 to 19.7 dB | 14.4 dB | 13.7 dB | 0.67 dB |

## The session-time model behind C.4

RP-3 §12's five turns of 201, 530, 345, 1 351 and 38 bytes, with 8 % framing. A call and an acceptance of 0.35 s each; per transmission, a 0.08 s preamble and a 0.12 s header or answer; a gateway think time of 0.5 s after `HELLO` and 0.2 s after `REQUEST`; the caller's disconnect riding with its last turn, the slave's last answer carrying the acknowledgement of it; no Morse identifier. Large transfers: 8 s bursts, each answered.
