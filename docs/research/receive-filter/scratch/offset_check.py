"""Does R4's per-bin 20th percentile, over looks divided by their own span median, raise the bins
a signal holds above their neighbours when that signal covers a fifth of the span? A synthetic
check: a sloped radio (flat to 1 kHz, 4.5 dB/kHz down above), exponential noise, a 500 Hz block
8 dB over the noise at 1050-1550 Hz on in two looks of three; no leave-out (the block is wider
than it reaches). R4's profile and drops against the same without the block."""
import numpy as np

rng = np.random.default_rng(1)
bin_hz = 48000 / 4096
hz = np.arange(0, 4000, bin_hz)
span = (hz >= 360) & (hz <= 2640)
slope = -4.5 * np.maximum(hz - 1000, 0) / 1000
block = (hz >= 1050) & (hz <= 1550)


def drops(block_on_share, q_time):
    looks = []
    for k in range(120):
        p = 10 ** (slope / 10) * rng.exponential(size=hz.size)
        if rng.random() < block_on_share:
            p = p + block * 10 ** (8 / 10)
        db = 10 * np.log10(p)
        looks.append(db - np.median(db[span]))
    prof = np.quantile(np.array(looks), q_time, axis=0)
    half = round(50 / bin_hz)
    sm = np.array([np.median(prof[max(0, i - half):i + half + 1]) for i in range(prof.size)])
    ref = np.quantile(sm[span], 0.9)
    lo, hi = np.flatnonzero(span)[[0, -1]]
    return ref - sm[lo], ref - sm[hi], np.median(sm[block]) - np.median(sm[(hz > 400) & (hz < 1000)])


for share in (0.0, 0.66):
    for q in (0.5, 0.2):
        lo, hi, bump = drops(share, q)
        print(f"block on {share:.0%} time quantile {q}: bottom {lo:5.1f} dB, top {hi:5.1f} dB, block bins over the flat part {bump:+.1f} dB")
