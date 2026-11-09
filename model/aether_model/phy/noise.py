"""The noise across the band, read where the signal is (ADR-0104).

A radio's receive filter shapes the band's noise as it shapes the signal: a carrier or a tone
its skirt cuts by 20 dB has 20 dB less noise too, and keeps its SNR as long as the audio chain's
own floor is further down. The receivers took the noise as flat across their band — the OFDM
receiver charged every data carrier one noise per symbol over its own ``|h|²``, the tone
receiver read every tone against one noise level — so a carrier or a tone in the skirt read as
one 20 dB under the others: the OFDM decoder all but discarded it, and the tone decoder took a
symbol sent on it for the uncut tones' noise. On the mobile's receiver of 2026-11-07 (its IF
shift turned fully left: 16 dB down at 1.8 kHz, 23 dB from 2 kHz) that cost every 2300 Hz OFDM
rung from 13 up at 3 dB over its threshold, and a passband 20 dB down at the 500 Hz signal's
edge cost every tone rung (the receive-filter review, ``docs/research/receive-filter/
02-survey.md`` §4; ``tools/bench_cut.py``).

Each receiver now reads the noise per pilot carrier or per tone from the bins that should hold
only noise, and :func:`shaped_levels` turns those readings into a shape: smoothed over three
neighbours, and pulled toward one level by how little they differ beyond the scatter of their
own estimates. Where nothing shapes the band the readings differ by that scatter alone and the
shape is dropped — the receiver is the flat one it was (``tools/bench_noise_shape.py``: on
AWGN, Good, Moderate, Poor and flutter, both airs, 1681 OFDM frames decoded of the 1682 the
flat receiver decoded, with the shape smoothed but not pulled, 1671).
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

FloatArray = NDArray[np.float64]


def shaped_levels(level: FloatArray, scatter: float, centre: float) -> FloatArray:
    """Noise levels in nepers across the band, one a pilot carrier or a tone, from their raw
    readings ``level``: smoothed over three neighbours (the ends repeated), and pulled toward
    ``centre`` by an empirical-Bayes factor, ``1 − scatter / var(level)`` at least 0, where
    ``scatter`` is the variance each reading's own estimate has. Fewer than three readings
    give ``centre`` everywhere."""
    if len(level) < 3:
        return np.full(len(level), centre, dtype=np.float64)
    smooth = np.convolve(np.pad(level, 1, mode="edge"), [0.25, 0.5, 0.25], mode="valid")
    keep = max(0.0, 1.0 - scatter / max(float(np.var(level)), 1e-12))
    return np.asarray(centre + keep * (smooth - centre), dtype=np.float64)
