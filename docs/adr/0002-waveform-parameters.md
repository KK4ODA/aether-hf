# ADR-0002: Aether HF v1 waveform — OFDM numerology and starting parameters

**Status:** accepted as *starting point* (2026-09-13); to be confirmed or amended by Phase 1
simulation (P1-4 … P1-7) · **Code:** `model/aether_model/waveform.py` · **Roadmap:** §5.2

## Context

The legacy prototype used a 12 kHz complex baseband with a 256-point FFT (46.875 Hz
spacing), 64 carriers (3 000 Hz — wider than its own "2 300 Hz" mode), a 3.2 ms cyclic
prefix, a pilot grid that left edge carriers extrapolated, and a mode table whose rates did
not follow from any of it. None of it had been simulated. A waveform has to be fixed early
because the frame format, ARQ timing, mode table and the air-interface specification all
derive from it.

## Decision

Keep **pilot-aided OFDM** for v1 (rationale in `docs/ROADMAP.md` §5.1) with this numerology:

| Parameter | Value | Reason |
|---|---|---|
| Audio I/O | 48 kHz (44.1/96 accepted via resampler) | universal |
| Complex baseband | 8 kHz, centre 1 500 Hz | 6:1 integer resampling; 3 kHz SSB channel; conventional audio placement |
| FFT size / spacing | 200 / 40 Hz | useful symbol 25 ms; ICI ≈ f_d/Δf ≤ 5 % at 2 Hz Doppler |
| Cyclic prefix | 48 samples = 6 ms | covers ITU Poor (2 ms) with 3× margin; NVIS (7 ms) via an extended-CP mode option |
| Symbol | 248 samples = 31 ms, 32.26 Bd | |
| Carriers | 2 300 Hz: 57 · 2 750 Hz: 68 · 500 Hz: 12 | occupied 2 280 / 2 720 / 480 Hz; matches VARA's BW500/2300/2750 host options; US HF limit is 2.8 kHz |
| Pilots | comb every 4th carrier on every symbol, both band edges always pilots (15 of 57), plus a full pilot symbol every 8th symbol | 2-D channel interpolation never extrapolates; tracks Poor-channel Doppler |
| Modulations | BPSK, QPSK, 8-PSK, 16-QAM, 64-QAM; Gray-labelled; BICM | 32/128/256-QAM dropped as unrealistic on fading HF |
| Windowing | raised-cosine with overlap-add (symbol extended by the taper) + polyphase TX filter | spectral mask without the ICI the legacy code inflicted on itself |
| TX envelope | preamble PAPR ≤ data PAPR; constant average power across preamble / data / ACK; soft ramps | field complaint about ALC spikes (`COMMUNITY-CONCERNS.md` #8) |
| Preamble | 2 identical PN OFDM symbols (Schmidl–Cox timing + fractional CFO) + 1 unique-word symbol (integer CFO, frame type, bandwidth), band-limited to the data bandwidth | ±250 Hz acquisition without CAT; deterministic timing for ARQ |
| Tracking | SRO from pilot phase slope across carriers/time; phase per symbol from pilots; timing re-lock per frame | sound cards differ by ±100–200 ppm |

Derived raw payload rates (2 300 Hz, from `WaveformParams.raw_bit_rate`): BPSK 1/5 ≈ 237 bps,
QPSK ½ ≈ 1 185 bps, 16-QAM ¾ ≈ 3 556 bps, 64-QAM ⅚ ≈ 5 927 bps, before framing/ARQ overhead.

## Alternatives considered

- **Keep 12 kHz / N=256** (legacy): no advantage; 6:1 to 8 kHz is cheaper and matches
  codec2 tooling for comparison.
- **N=256 at 8 kHz (31.25 Hz spacing, 32 ms symbols)**: lower CP overhead but ICI at 10 Hz
  flutter reaches ~30 %; 40 Hz is the compromise. Revisit if flutter matters in the field.
- **Sparser pilots (every 8th carrier, pilot symbol every 4th)**: ~13 % overhead instead of
  ~34 %; deferred to a Phase 2 experiment once Poor-channel curves exist to compare against.
- **Single-carrier serial-tone (MIL-STD-188-110 style)** and **DFT-spread OFDM**: see
  roadmap §5.1; the PHY is behind a trait so either can become "HF PHY v2".

## Consequences

- `WaveformParams` is the single source of numerology; the spec, mode table and tests derive
  from it (`test_waveform.py` pins the values above).
- The 25 % comb-pilot overhead is a deliberate robustness-first choice; the throughput cost
  is measured, not assumed, in Phase 2.
- Any change to this table requires amending this ADR and the pinned tests together.

## Amendments (2026-09-13, Phase 1 implementation)

Recorded as the model was built and measured (`model/aether_model/phy/`, `frame/`):

- **Preamble sequences are PN, not Zadoff–Chu.** A ZC chirp's frequency shift is (up to
  phase) a time shift, so neither the Schmidl–Cox matched filter nor a differential
  unique-word detector could separate carrier offset from timing — the same flaw the audit
  found in the legacy detector. The SC symbols carry a fixed PN sequence on the even
  carriers (`SC_SEED`); the unique word is one of 32 PN sequences selected for low mutual
  and shifted correlation, and its index is the frame header (14 data modes + control).
  Full pilot symbols keep the Zadoff–Chu sequence (root 7) for its low PAPR (≈ 3.5 dB).
- **Header and integer CFO** are detected from three channel-blind statistics (SC2→UW
  ratio, UW adjacent-carrier differential, UW→pilot ratio); the 80 Hz ambiguity of the
  half-symbol CFO estimate is resolved by matched-filter hypothesis testing.
- **Window taper** 8 samples (1 ms); effective CP 5 ms.
- **Frame layouts:** LONG = 3 + 32 symbols (1.09 s, 28 payload symbols × 42 carriers);
  SHORT (control) = 3 + 12 symbols (0.47 s, 7 payload bytes at BPSK 1/5).
- **Mode table** (14 modes, `frame/modes.py`): BPSK 1/5 … 64-QAM 5/6, one LDPC block per
  frame with CRC-24A; base graph per TS 38.212 §7.2.2.
- **Interleaver:** coprime-stride permutation `k·p mod E`, `p ≈ E/φ`.
- **Measured AWGN thresholds** (FER < 5 %, 3 kHz SNR, random ±100 Hz CFO and ±50 ppm SRO,
  `bench/baselines/phy_fer.csv`): BPSK ½ −1 dB, QPSK ½ +2, 8-PSK ½ +5, 16-QAM ½ +7,
  16-QAM ¾ +10, 64-QAM ⅚ +17. BPSK 1/5 is acquisition-limited at −3 dB: the decoder
  works to ≈ −6 dB but the preamble detector does not yet (roadmap P2-3).

## Amendments (2026-09-13, Phase 2 / P2-3 — low-SNR acquisition)

Measurement showed the unique-word header, not the preamble, was the low-SNR bottleneck:
a coherent 57-chip header metric sits at the noise level at −7 dB (3 kHz) whatever the
channel estimate. The air interface was changed so that nothing weak is on the critical path:

- **Preamble = two Schmidl–Cox symbols only** (LONG frame 34 symbols ≈ 1.05 s, SHORT 14
  symbols ≈ 0.43 s). The unique-word symbol is gone.
- **Frame type is carried by the SC sequence** (`SC_SEEDS`: DATA / CONTROL). The receiver's
  matched-filter bank correlates against both, so the type decision has the full preamble
  processing gain behind it.
- **Mode is carried as PN chips on the data carriers of the full pilot symbols** of DATA
  frames (4 × 42 = 168 chips; 14 sequences with pairwise |ρ| ≤ 0.2, `MODE_CHIP_SEED`).
  The receiver estimates the channel from the comb pilots of those symbols, correlates
  the chips coherently, then treats the pilot symbols as fully known. If the CRC fails
  and the chip metric ratio was below 1.3, the runner-up mode is tried once. Control
  frames use the fixed control mode; their pilot symbols carry the plain pilot sequence.
- **Acquisition is a partial-matched-filter/FFT bank** (62 segments of 8 samples, 256-bin
  FFT: ±300 Hz at 3.9 Hz) evaluated at every position, followed by a fine-grid (0.24 Hz)
  evaluation at the winning position and a full-symbol-lag refinement. Threshold 0.36 on
  the normalised peak (noise maximum 0.33 over 60 s; 0 false alarms). Detections are
  suppressed inside an accepted frame's span and one symbol before a stronger peak (the
  identical SC symbols produce a 0.7 sidelobe one symbol early).
- **Measured** (AWGN, random ±250 Hz CFO): acquisition 100 % at −5 dB, 97 % at −6 dB,
  87 % at −7 dB, 70 % at −8 dB; type and mode correct in every acquired frame down to
  −10 dB; BPSK 1/5 decodes 100 % at −5 dB and ~50 % at −6 dB — the code, not the
  acquisition, is now the floor.
