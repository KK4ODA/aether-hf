# AETHER HF

**Advanced Extensible Transceiver for HF Environments and Resilient Links**

Open-source HF modem protocol for amateur radio digital communications. Designed to outperform VARA HF at low-to-moderate SNR while remaining fully open, cross-platform, and implementable on commodity hardware.

## Key Advantages Over VARA HF

| Feature | VARA HF | AETHER HF |
|---------|---------|-----------|
| Low-SNR threshold | 0 dB | -6 dB (-10 dB emergency) |
| ACK frame duration | 842 ms | 172 ms (75% reduction) |
| FEC | Turbo codes | LDPC (0.5-1.5 dB closer to Shannon limit) |
| Detection | Differential PSK (levels 1-8) | Coherent on all levels (+2-3 dB) |
| Retransmission | Full retransmit | HARQ-IR incremental redundancy (+2-4 dB) |
| Frequency acquisition | Narrow | +/-250 Hz (no CAT required) |
| Speed levels | 16 wide / 13 narrow | 20 wide / 14 narrow |
| Platform | Windows only | Cross-platform (Python prototype) |
| License | Proprietary | MIT / Apache-2.0 |

## Architecture

```
aether_hf/
  dsp/
    ofdm.py         OFDM modulator/demodulator with pilot-based equalization
    modulation.py   BPSK through 256-QAM constellation mappers + soft LLR demapping
    preamble.py     Zadoff-Chu preamble with +/-250 Hz coarse CFO sweep
  fec/
    ldpc.py         LDPC encoder + belief-propagation decoder (min-sum)
    interleaver.py  Three-stage frequency/time/bit interleaving
  protocol/
    session.py      ARQ state machine, HARQ-IR, rate adaptation
  host/
    vara_compat.py  VARA-compatible TCP command/data server
  audio/
    soundcard.py    Sound card I/O with 48 kHz to 12 kHz baseband resampling
  constants.py      All protocol parameters from the v2.0 specification
  speed_levels.py   20 wide + 14 narrow speed level definitions
```

## Quick Start

```bash
pip install numpy scipy

# Run the OFDM loopback test
python -m aether_hf.tests.test_ofdm_loopback
```

Expected output:
```
Constellation mapping tests:
      BPSK: PASS
      QPSK: PASS
     8-PSK: PASS
    16-QAM: PASS
    64-QAM: PASS
No noise:   0 bit errors / 104 bits

AWGN test (QPSK, 52 carriers):
  SNR (dB)        BER     Errors
-----------------------------------
        20   0.000000          0
        15   0.000000          0
        10   0.000000          0
         5   0.007692         16
         0   0.064423        134

Preamble detection: detected=True, ...
All tests passed!
```

## VARA Compatibility

AETHER HF exposes a TCP socket interface identical to VARA's protocol. Applications like Winlink Express, PAT, and VarAC can connect without modification:

| VARA Command | AETHER Mapping |
|-------------|---------------|
| `MYCALL <call>` | Sets local callsign |
| `CONNECT <call>` | Initiates AETHER session |
| `DISCONNECT` | Sends QRT, closes session |
| `LISTEN ON/OFF` | Enables/disables monitoring |
| `BUFFER` | Reports TX buffer size |
| `BW500 / BW2300` | Selects narrow/wide mode |

Default ports: command 8400, data 8401 (configurable).

## Speed Levels (Wide Mode, 2300 Hz)

| Level | Modulation | Code Rate | Net Rate (bps) | Min SNR (dB) |
|-------|-----------|-----------|---------------|-------------|
| 1 | 2-FSK | 1/8 | 12 | -10 |
| 4 | BPSK | 1/4 | 210 | -1 |
| 7 | QPSK | 1/2 | 850 | +6 |
| 12 | 8-PSK | 3/4 | 1,910 | +15 |
| 15 | 16-QAM | 3/4 | 2,550 | +18 |
| 18 | 64-QAM | 3/4 | 3,825 | +25 |
| 20 | 256-QAM | 5/6 | 5,700 | +31 |

## Status

This is a **Python prototype**. The DSP core, FEC engine, protocol state machine, and host interface are functional. A production C++ implementation with FFTW3 and AFF3CT would follow for real-time on-air use.

**What works:**
- OFDM modulation/demodulation with channel estimation
- All constellation mappers (BPSK through 256-QAM) with soft demapping
- Preamble generation, detection, and CFO estimation
- LDPC encoding and belief-propagation decoding
- Frequency/time/bit interleaving
- Session state machine with rate adaptation
- VARA-compatible TCP host interface
- Audio I/O with resampling and loopback testing

**What's next:**
- End-to-end frame encode/decode pipeline
- HARQ-IR soft combining across retransmissions
- Emergency Mode (DSSS single-carrier)
- On-air testing with real HF transceivers
- C++ production implementation

## Integration with VARA BBS

AETHER HF is a transport option in [VARA BBS](https://github.com/KK4ODA/vara-bbs). Configure an instance with `transport = "AETHER_HF"` and the BBS connects to AETHER's VARA-compatible TCP ports.

## Specification

The full technical design proposal (v2.0) is included as a Word document in this repository. It covers channel modeling, waveform design, FEC, synchronization, adaptive link control, noise mitigation, protocol design, VARA compatibility, implementation architecture, performance analysis, conformance testing, and security considerations.

## Dependencies

- Python 3.9+
- numpy
- scipy
- sounddevice (optional, for real audio I/O)

## License

MIT / Apache-2.0 dual license.

## Contributors

- **KK4ODA** (Facundo) — Author
- **Claude** (Anthropic) — Co-developer
