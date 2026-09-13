"""Impulsive-noise blanker for the receiver front end (roadmap P2-5).

HF is full of impulsive noise — ignition, switching supplies, plasma TVs, static crashes.
It is the one impairment OFDM handles *worse* than a single-carrier waveform: the FFT spreads
a single hot sample across all 57 carriers of the symbol it lands in, so one microsecond of
interference damages a whole 31 ms symbol. The defence is to remove the impulse in the time
domain, before the FFT ever sees it.

Two things have to be true for that to work, and both dictate where this sits in the chain:

* **Blank before band-limiting.** The receive filter smears an impulse into a long ringing
  tail; once that has happened there is no longer a small set of hot samples to remove. This
  runs on the raw stream, ahead of :meth:`~aether_model.phy.sync.FrameDetector.condition`.
* **Judge against a robust envelope.** The threshold has to come from a statistic the
  impulses themselves cannot drag upward, or a strong burst raises the bar until it no
  longer trips it. A moving *median* of the envelope is used, not a moving mean.

Blanking is not free: zeroing a sample removes signal along with noise, and blanking too
eagerly is its own impairment. The default threshold is set where clean Gaussian noise is
essentially never blanked (see :attr:`NoiseBlanker.threshold_sigma`), so on a quiet channel
the blanker does nothing at all.

What blanking leaves behind is handled downstream: the receiver estimates noise variance per
OFDM symbol, so a symbol that still took damage gets its LLRs scaled down and effectively
becomes an erasure the LDPC decoder can work around.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import ndimage

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]
BoolArray = NDArray[np.bool_]

_RAYLEIGH_MEDIAN = 1.1774100225154747
"""median(|x|) / sigma for a complex Gaussian with per-component variance sigma^2:
sqrt(2 ln 2). Converts a robust median envelope into the RMS the threshold is set from."""


@dataclass
class BlankerResult:
    samples: ComplexArray
    blanked: BoolArray
    """True where a sample was removed."""

    @property
    def fraction(self) -> float:
        return float(np.mean(self.blanked)) if self.blanked.size else 0.0


class NoiseBlanker:
    """Median-referenced impulse blanker.

    ``threshold_sigma`` is in units of the local RMS envelope. For complex Gaussian noise the
    envelope is Rayleigh, so the probability a *clean* sample is blanked is ``exp(-k^2)``:
    1.2e-4 at k = 3, 4.8e-6 at k = 3.5, 1.1e-7 at k = 4. The default of 3.5 costs a quiet
    channel essentially nothing while still catching bursts tens of dB above the noise.
    """

    def __init__(
        self,
        threshold_sigma: float = 3.5,
        window: int = 101,
        segment_span: int = 9,
        mode: str = "blank",
    ) -> None:
        if threshold_sigma <= 0:
            raise ValueError("threshold must be positive")
        if mode not in ("blank", "clip"):
            raise ValueError("mode must be 'blank' or 'clip'")
        self.threshold_sigma = float(threshold_sigma)
        self.window = max(3, int(window))
        self.segment_span = max(1, int(segment_span)) | 1  # odd, so the window is centred
        self.mode = mode

    @property
    def robust_span_samples(self) -> int:
        """Longest burst the reference can survive: it has to stay a minority of the segments
        the running median looks at. ≈ 57 ms at 8 kHz with the defaults."""
        return self.window * self.segment_span // 2

    def envelope_rms(self, x: ComplexArray) -> FloatArray:
        """Local RMS envelope, estimated as a *median of segment medians*.

        A single moving median is robust only to impulses that are a minority inside its own
        window. A static crash lasting tens of milliseconds is longer than any window short
        enough to track fading, and inside such a burst the local median *is* the burst — the
        threshold rises with it and the blanker sails straight past the thing it exists to
        catch. Taking the median of per-segment medians pushes the robustness out to
        :attr:`robust_span_samples` while keeping the estimator cheap and still fast enough
        to follow fading, which moves at 0.1–1 Hz.
        """
        mag = np.abs(np.asarray(x, dtype=np.complex128))
        n = len(mag)
        if n == 0:
            return np.zeros(0, dtype=np.float64)
        w = min(self.window, n)
        n_seg = n // w
        if n_seg < 2:
            level = np.full(n, float(np.median(mag)))
        else:
            seg_med = np.median(mag[: n_seg * w].reshape(n_seg, w), axis=1)
            smooth = ndimage.median_filter(
                seg_med, size=min(self.segment_span, n_seg), mode="nearest"
            )
            level = np.repeat(smooth, w)
            if len(level) < n:  # the ragged tail keeps the last segment's level
                level = np.concatenate((level, np.full(n - len(level), level[-1])))
        sigma = level / _RAYLEIGH_MEDIAN  # per-component sigma
        return np.asarray(sigma * np.sqrt(2.0), dtype=np.float64)  # RMS of the complex sample

    def process(self, x: ComplexArray) -> BlankerResult:
        y = np.asarray(x, dtype=np.complex128).copy()
        if y.size == 0:
            return BlankerResult(y, np.zeros(0, dtype=bool))
        rms = self.envelope_rms(y)
        threshold = self.threshold_sigma * rms
        mag = np.abs(y)
        hot = (mag > threshold) & (threshold > 0)
        if self.mode == "blank":
            y[hot] = 0.0
        else:
            scale = np.ones_like(mag)
            scale[hot] = threshold[hot] / mag[hot]
            y = y * scale
        return BlankerResult(y, hot)
