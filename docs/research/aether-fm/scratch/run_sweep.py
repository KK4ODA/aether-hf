import json
import sys

import numpy as np

from fmchain import FS, Radio, aether_hf, measure, ofdm, sc

D = 6.0
rng = np.random.default_rng(11)

MIC = Radio("mic", dev_hz=5000.0, if_half_hz=7500.0)  # 25 kHz channel, +-5 kHz, 15 kHz IF
NARROW = Radio("mic", dev_hz=2500.0, if_half_hz=4250.0)  # 12.5 kHz channel, +-2.5 kHz
DATA = Radio("data", dev_hz=3000.0, if_half_hz=7500.0)  # flat 9600-style port, 15 kHz IF

mic_w = {
    "OFDM": (ofdm(450, 2850, 40, 0.006, 16, D, rng), (450, 2850), False),
    "DFT-s-OFDM": (ofdm(450, 2850, 40, 0.006, 16, D, rng, dfts=True), (450, 2850), False),
    "SC 2000Bd": (sc(2000, 1650, 0.20, 16, D, rng), (450, 2850), False),
    "SC 2000Bd +precomp": (sc(2000, 1650, 0.20, 16, D, rng), (450, 2850), True),
    "OFDM +precomp": (ofdm(450, 2850, 40, 0.006, 16, D, rng), (450, 2850), True),
    "AetherHF2300": (aether_hf(D, rng, True), (400, 2600), False),
}
data_w = {
    "OFDM 300-5100": (ofdm(300, 5100, 40, 0.006, 16, D, rng), (300, 5100), False),
    "SC 4000Bd": (sc(4000, 2700, 0.20, 16, D, rng), (300, 5100), False),
}

cases = [("mic25", MIC, mic_w), ("mic12.5", NARROW, {k: mic_w[k] for k in ("OFDM", "SC 2000Bd", "SC 2000Bd +precomp")}), ("data25", DATA, data_w)]
cnrs = [30, 18, 12, 9]
drives = np.arange(-24.0, 4.6, 1.5)

out = []
for cname, radio, ws in cases:
    for cnr in cnrs:
        for wname, (x, band, pre) in ws.items():
            for dv in drives:
                nrng = np.random.default_rng(int(1000 * cnr + 10 * dv + 5000))
                m = radio.tx(x, float(dv), pre)
                d = radio.air(m, cnr, nrng)
                r = radio.rx(d, nrng)
                rate, eff = measure(x, r, band)
                out.append(dict(case=cname, cnr=cnr, wave=wname, drive=float(dv), rate=rate, sndr=eff))
        print(cname, cnr, "done", file=sys.stderr)

json.dump(out, open("sweep.json", "w"))
