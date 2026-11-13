"""Two-level probe: SNDR of short segments at drive d-1.5 and d+1.5 dB (each its own 0.25 s of
16-QAM SC data, data-aided LS fit), the sign of the difference against the true side of the
optimum found by the long sweep. 40 trials per point."""
import numpy as np, scipy.signal as sg
from fmchain import FS, Radio, sc
MIC = Radio("mic", dev_hz=5000.0, if_half_hz=7500.0)
L = 64
def sndr(x, r):
    lag = np.argmax(sg.correlate(r, x, mode="full", method="fft")) - (len(x) - 1)
    xs = np.roll(x, lag); idx = np.arange(L, len(x) - L)
    X = np.stack([xs[idx + k] for k in range(-L//2, L//2)], axis=1)[::2]; y = r[idx][::2]
    h, *_ = np.linalg.lstsq(X, y, rcond=None); e = y - X @ h
    return 10*np.log10(np.mean((X@h)**2)/np.mean(e**2))
opt = {30: -7.5, 18: -7.5, 9: -6.0}  # SC +precomp, from the long sweep
rng = np.random.default_rng(5)
print(f"{'CNR':>4s} {'drive-opt':>9s} {'mean dSNDR':>10s} {'sd':>5s} {'P(move toward opt)':>19s}")
for cnr in (30, 18, 9):
    for off in (-9, -6, -3, -1.5, 0, 1.5, 3, 6):
        d0 = opt[cnr] + off; diffs = []
        for t in range(40):
            seg = []
            for dv in (d0 - 1.5, d0 + 1.5):
                x = sc(2000, 1650, 0.20, 16, 0.25, rng)
                nr = np.random.default_rng(rng.integers(1 << 30))
                r = MIC.rx(MIC.air(MIC.tx(x, dv, True), cnr, nr), nr)
                seg.append(sndr(x, r))
            diffs.append(seg[1] - seg[0])
        diffs = np.array(diffs)
        want = np.sign(-off) if off != 0 else 0
        p = np.mean(np.sign(diffs) == want) if want != 0 else np.mean(np.abs(diffs) < 1.5)
        print(f"{cnr:4d} {off:9.1f} {diffs.mean():10.2f} {diffs.std():5.2f} {p:19.2f}")
