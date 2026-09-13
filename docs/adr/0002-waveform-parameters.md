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
