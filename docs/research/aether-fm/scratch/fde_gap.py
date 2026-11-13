"""Penalty of a linear MMSE frequency-domain equaliser (single carrier) against per-frequency
coding (OFDM's ideal), on the measured SNDR(f) of the radio chain at the optimum drive."""
import numpy as np, scipy.signal as sg
from fmchain import FS, Radio, sc
rng = np.random.default_rng(9)
cases = [("mic25", Radio("mic", 5000.0, 7500.0), sc(2000, 1650, 0.2, 16, 6.0, rng), (850, 2450), True, -7.5),
         ("mic25 no precomp", Radio("mic", 5000.0, 7500.0), sc(2000, 1650, 0.2, 16, 6.0, rng), (850, 2450), False, -7.5),
         ("data25", Radio("data", 3000.0, 7500.0), sc(4000, 2700, 0.2, 16, 6.0, rng), (1100, 4300), False, 0.0)]
for name, radio, x, band, pre, dv in cases:
    for cnr in (30, 18, 9):
        nr = np.random.default_rng(cnr)
        r = radio.rx(radio.air(radio.tx(x, dv, pre), cnr, nr), nr)
        lag = np.argmax(sg.correlate(r[:2*FS], x[:2*FS], mode="full", method="fft")) - (2*FS-1)
        r2, x2 = (r[lag:], x[:len(x)-lag]) if lag > 0 else (r, x)
        f, C = sg.coherence(x2, r2, fs=FS, nperseg=4096, noverlap=2048)
        s = (f >= band[0]) & (f <= band[1]); C = np.clip(C[s], 0, 1-1e-6); snr = C/(1-C)
        geo = 10*np.log10(2**np.mean(np.log2(1+snr)) - 1)          # per-frequency coding / MMSE-DFE
        lin = 10*np.log10(1/np.mean(1/(1+snr)) - 1)                # linear MMSE equaliser
        print(f"{name:18s} CNR {cnr:2d}: SNDR(f) spread {10*np.log10(snr.min()):5.1f}..{10*np.log10(snr.max()):5.1f} dB, "
              f"per-f coding {geo:5.1f} dB, linear MMSE {lin:5.1f} dB, gap {geo-lin:4.2f} dB")
