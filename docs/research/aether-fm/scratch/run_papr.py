import numpy as np

from fmchain import aether_hf, cpfsk4, ofdm, papr_db, sc

rng = np.random.default_rng(7)
D = 20.0
rows = [
    ("Aether HF 2300 OFDM, 16QAM mode, ADR-0004 clip-and-filter", aether_hf(D, rng, True)),
    ("Aether HF 2300 OFDM, 16QAM mode, no peak reduction", aether_hf(D, rng, False)),
    ("CP-OFDM 450-2850 Hz, 40 Hz carriers, 16QAM", ofdm(450, 2850, 40, 0.006, 16, D, rng)),
    ("DFT-s-OFDM 450-2850 Hz, 16QAM (no spectral shaping)", ofdm(450, 2850, 40, 0.006, 16, D, rng, dfts=True)),
    ("SC 2000 Bd RRC a=0.20, QPSK", sc(2000, 1650, 0.20, 4, D, rng)),
    ("SC 2000 Bd RRC a=0.20, 16QAM", sc(2000, 1650, 0.20, 16, D, rng)),
    ("SC 2000 Bd RRC a=0.20, 64QAM", sc(2000, 1650, 0.20, 64, D, rng)),
    ("SC 2000 Bd RRC a=0.35, 16QAM", sc(2000, 1650, 0.35, 16, D, rng)),
    ("SC 4000 Bd RRC a=0.20, 16QAM (data port)", sc(4000, 2700, 0.20, 16, D, rng)),
    ("4-CPFSK 1200 Bd (constant envelope)", cpfsk4(1200, 1700, 200, D, rng)),
]
print(f"{'waveform':60s} {'1e-3':>6s} {'1e-4':>6s}  (dB over RMS, real audio)")
for name, x in rows:
    print(f"{name:60s} {papr_db(x, 1e-3):6.2f} {papr_db(x, 1e-4):6.2f}")
