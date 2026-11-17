"""Mean power spectrum of stretches of a recording (taken every 0.1 s), and each stretch's R3
reading as a profile of its own looks (every 0.5 s).

    python -I segments.py OUT_PNG WAV BW FROM-TO [FROM-TO ...]
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import matplotlib  # noqa: E402

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402

import r3  # noqa: E402


def main():
    out, wav, bw = sys.argv[1], sys.argv[2], int(sys.argv[3])
    span = r3.SPANS[bw]
    audio = r3.read_wav(wav)
    hz = np.arange(int(10_000 / r3.BIN_HZ) + 1) * r3.BIN_HZ
    fig, (ax, bx) = plt.subplots(2, 1, figsize=(12, 9))
    for seg in sys.argv[4:]:
        a, b = (float(x) for x in seg.split("-"))
        acc, n = None, 0
        looks = []
        for t in np.arange(a + 0.1, b + 1e-9, 0.1):
            spec = r3.spectrum_at(audio, t)
            p = 10 ** (spec.astype(float) / 10)
            acc = p if acc is None else acc + p
            n += 1
        for t in np.arange(a + 0.1, b + 1e-9, 0.5):
            look, over, why = r3.fold(r3.spectrum_at(audio, t), span)
            if look is not None:
                looks.append(look)
        mean = 10 * np.log10(acc / n)
        ax.plot(hz, mean, lw=1, label=f"{a:g}-{b:g} s mean power")
        if looks:
            prof = r3.profile(looks)
            sm = np.array([r3.smoothed_at(prof, k) if r3.smoothed_at(prof, k) is not None else np.nan for k in range(len(prof))])
            d = r3.span_drops(prof, span)
            first, last = r3.rround(span[0] / r3.BIN_HZ), r3.rround(span[1] / r3.BIN_HZ)
            ref = r3.quantile([v for v in sm[first : last + 1] if np.isfinite(v)], r3.REF_Q)
            bx.plot(hz[: len(sm)], sm - ref, lw=1, label=f"{a:g}-{b:g} s: {len(looks)} looks, low {d[0]:.1f} high {d[1]:.1f}")
    for axis in (ax, bx):
        for x in span:
            axis.axvline(x, color="k", lw=0.5, ls=":")
        axis.set_xlim(0, 4000)
        axis.grid(alpha=0.3)
        axis.legend(fontsize=7)
    ax.set_ylabel("dBFS a bin")
    bx.set_ylabel("R3 profile, dB under the span's ninth decile")
    bx.axhline(-12, color="r", lw=0.5)
    bx.set_ylim(-35, 6)
    bx.set_xlabel("Hz")
    fig.tight_layout()
    fig.savefig(out, dpi=100)


main()
