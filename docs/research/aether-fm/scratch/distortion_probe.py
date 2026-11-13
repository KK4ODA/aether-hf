"""Can one transmission tell clipping from noise? Fit r = h*x (LS FIR), split the residual
by the modem signal's envelope, compare error power at the peaks with error power elsewhere."""
import numpy as np, scipy.signal as sg
from fmchain import FS, Radio, sc

rng = np.random.default_rng(3)
x = sc(2000, 1650, 0.20, 16, 6.0, rng)
env = np.abs(sg.hilbert(x))
MIC = Radio("mic", dev_hz=5000.0, if_half_hz=7500.0)
L = 96
def resid(x, r):
    lag = np.argmax(sg.correlate(r[:2*FS], x[:2*FS], mode="full", method="fft")) - (2*FS-1)
    xs = np.roll(x, lag)
    n = len(x); idx = np.arange(L, n - L)
    # LS FIR from x (centred taps) to r
    X = np.stack([xs[idx + k] for k in range(-L//2, L//2)], axis=1)[::3]
    y = r[idx][::3]
    h, *_ = np.linalg.lstsq(X, y, rcond=None)
    e = y - X @ h
    return e, np.roll(env, lag)[idx][::3], X @ h
print(f"{'CNR':>4s} {'drive':>6s} {'SNDR':>6s} {'err@peaks/err@median (dB)':>26s} {'D/N est (dB)':>12s}")
for cnr in (30, 18, 9):
    for dv in (-15.0, -10.5, -7.5, -6.0, -4.5, -1.5):
        nrng = np.random.default_rng(int(cnr*100+dv*10+77))
        r = MIC.rx(MIC.air(MIC.tx(x, dv, True), cnr, nrng), nrng)
        e, ev, fit = resid(x, r)
        q = np.quantile(ev, [0.4, 0.6, 0.9])
        mid = np.mean(e[(ev > q[0]) & (ev < q[1])]**2)
        top = np.mean(e[ev > q[2]]**2)
        allp = np.mean(e**2); sndr = 10*np.log10(np.mean(fit**2)/allp)
        # crude split: noise = error where the envelope is near its median, distortion = the rest
        dn = 10*np.log10(max(allp/mid - 1, 1e-4))
        print(f"{cnr:4d} {dv:6.1f} {sndr:6.1f} {10*np.log10(top/mid):26.2f} {dn:12.1f}")
