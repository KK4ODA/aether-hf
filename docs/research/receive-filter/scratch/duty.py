"""How much of the time a QRM block was on: per look (every 0.5 s of a stretch), the median of
the block's bins over the median of the look's span (both dBFS), and the share of looks where it
stood 6 dB or more over.

    python -I duty.py WAV FROM TO BLOCK_LO BLOCK_HI [BW]
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np  # noqa: E402

import r3  # noqa: E402


def main():
    wav = sys.argv[1]
    a, b, lo, hi = (float(x) for x in sys.argv[2:6])
    bw = int(sys.argv[6]) if len(sys.argv) > 6 else 2300
    span = r3.SPANS[bw]
    audio = r3.read_wav(wav)
    excess = []
    for t in np.arange(a + 0.1, b + 1e-9, 0.5):
        spec = r3.spectrum_at(audio, t)
        level = float(np.median(r3.between(spec, *span)))
        if level < -100:
            continue  # a hop's silence
        excess.append(float(np.median(r3.between(spec, lo, hi))) - level)
    e = np.array(excess)
    print(
        f"{len(e)} looks: block {lo:.0f}-{hi:.0f} Hz over the span's median: median {np.median(e):.1f} dB, "
        f"quartiles {np.percentile(e, 25):.1f}/{np.percentile(e, 75):.1f}; on (>=6 dB) {100 * np.mean(e >= 6):.0f}%, "
        f">=10 dB {100 * np.mean(e >= 10):.0f}%"
    )


main()
