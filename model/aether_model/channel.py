"""HF channel simulator — the calibrated core of the Aether benchmark suite.

Conventions (these are the ones every benchmark number in this repository is quoted in):

* **SNR is defined in a 3 kHz noise bandwidth**, the MIL-STD-188-110 / codec2 convention used
  throughout the HF-modem literature: ``SNR = P_signal / (N0 * 3000 Hz)``. The simulator
  therefore adds white noise whose power *over the whole baseband sample rate* is
  ``P_signal * fs / (3000 * 10**(snr_db/10))``.
* **Doppler spread follows ITU-R F.1487**: the value is the *2σ* width of the Gaussian
  Doppler power spectrum, so a "1 Hz" channel has ``σ = 0.5 Hz``.
* **Every impairment is a streaming process** with internal state. Feeding a signal in
  blocks produces the same output as feeding it whole (``test_channel.py`` checks this), so
  the same simulator serves block benchmarks and real-time-style tests.
* **Fading has unit mean power** (``E|g|² = 1``) so the long-term received power equals the
  transmitted power and SNR keeps its meaning under fading. No per-block normalization
  is ever applied — deep fades are real fades.

Processing order in :class:`HfChannel` mirrors a real link::

    TX ─► saturating PA ─► Watterson multipath/fading ─► + interferer ─► + AWGN (+ impulsive)
       ─► RX frequency offset ─► RX sample-rate offset ─► RX

The previous implementation of this module (``dsp/channel.py``, removed) regenerated fading
per call, normalised each block, referenced SNR to 12 kHz and used σ where 2σ was meant;
see ``docs/AUDIT.md`` §4.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Literal

import numpy as np
from numpy.typing import NDArray
from scipy import signal

ComplexArray = NDArray[np.complex128]

SNR_REFERENCE_BANDWIDTH_HZ = 3000.0
"""Noise bandwidth that SNR figures are referenced to (HF-modem convention)."""


def complex_normal(rng: np.random.Generator, n: int) -> ComplexArray:
    """``n`` unit-power circular complex Gaussian samples.

    Real and imaginary parts are drawn *interleaved* from one call, so drawing ``n`` samples
    in one go or in consecutive chunks consumes the generator identically — this is what
    makes every impairment's output independent of how the input is blocked.
    """
    if n <= 0:
        return np.zeros(0, dtype=np.complex128)
    return rng.standard_normal(2 * n).view(np.complex128) / math.sqrt(2.0)


# ── Channel profiles ──────────────────────────────────────────────────


@dataclass(frozen=True)
class ChannelProfile:
    """A tapped-delay-line HF channel with Rayleigh-fading taps (Watterson model).

    ``doppler_spread_hz`` is the ITU-R F.1487 *2σ* spread of each tap's Gaussian Doppler
    spectrum. ``0.0`` means a static (non-fading) tap.
    """

    name: str
    delays_s: tuple[float, ...]
    doppler_spread_hz: tuple[float, ...]
    tap_powers_db: tuple[float, ...]

    def __post_init__(self) -> None:
        n = len(self.delays_s)
        if n == 0:
            raise ValueError("a channel profile needs at least one tap")
        if len(self.doppler_spread_hz) != n or len(self.tap_powers_db) != n:
            raise ValueError("delays_s, doppler_spread_hz and tap_powers_db must have equal length")
        if min(self.delays_s) < 0 or min(self.doppler_spread_hz) < 0:
            raise ValueError("delays and Doppler spreads must be non-negative")


def _two_ray(name: str, delay_s: float, spread_hz: float) -> ChannelProfile:
    return ChannelProfile(name, (0.0, delay_s), (spread_hz, spread_hz), (0.0, 0.0))


AWGN = ChannelProfile("AWGN (single static path)", (0.0,), (0.0,), (0.0,))

# ITU-R F.1487 Table 1 — mid-latitude (these coincide with the classic CCIR 520 good /
# moderate / poor channels used by MIL-STD-188-110 and codec2).
ITU_MID_QUIET = _two_ray("ITU-R F.1487 mid-latitude quiet (CCIR Good)", 0.5e-3, 0.1)
ITU_MID_MODERATE = _two_ray("ITU-R F.1487 mid-latitude moderate (CCIR Moderate)", 1.0e-3, 0.5)
ITU_MID_DISTURBED = _two_ray("ITU-R F.1487 mid-latitude disturbed (CCIR Poor)", 2.0e-3, 1.0)
ITU_MID_NVIS = _two_ray("ITU-R F.1487 mid-latitude disturbed NVIS", 7.0e-3, 1.0)
# Low and high latitudes, same recommendation.
ITU_LOW_QUIET = _two_ray("ITU-R F.1487 low-latitude quiet", 0.5e-3, 0.5)
ITU_LOW_MODERATE = _two_ray("ITU-R F.1487 low-latitude moderate", 2.0e-3, 1.5)
ITU_LOW_DISTURBED = _two_ray("ITU-R F.1487 low-latitude disturbed", 6.0e-3, 10.0)
ITU_HIGH_QUIET = _two_ray("ITU-R F.1487 high-latitude quiet", 1.0e-3, 0.5)
ITU_HIGH_MODERATE = _two_ray("ITU-R F.1487 high-latitude moderate", 3.0e-3, 10.0)
ITU_HIGH_DISTURBED = _two_ray("ITU-R F.1487 high-latitude disturbed", 7.0e-3, 30.0)
# CCIR "flutter fading" test channel (CCIR Report 549): fast fading, short delay.
CCIR_FLUTTER = _two_ray("CCIR flutter fading", 0.5e-3, 10.0)

PROFILES: dict[str, ChannelProfile] = {
    "awgn": AWGN,
    "good": ITU_MID_QUIET,
    "moderate": ITU_MID_MODERATE,
    "poor": ITU_MID_DISTURBED,
    "nvis": ITU_MID_NVIS,
    "flutter": CCIR_FLUTTER,
    "low-quiet": ITU_LOW_QUIET,
    "low-moderate": ITU_LOW_MODERATE,
    "low-disturbed": ITU_LOW_DISTURBED,
    "high-quiet": ITU_HIGH_QUIET,
    "high-moderate": ITU_HIGH_MODERATE,
    "high-disturbed": ITU_HIGH_DISTURBED,
}


def get_profile(profile: str | ChannelProfile) -> ChannelProfile:
    if isinstance(profile, ChannelProfile):
        return profile
    try:
        return PROFILES[profile.lower()]
    except KeyError:
        raise ValueError(
            f"unknown channel profile {profile!r}; available: {sorted(PROFILES)}"
        ) from None


# ── Fading process ────────────────────────────────────────────────────


class RayleighFadingProcess:
    """Unit-power complex Gaussian fading with a Gaussian Doppler spectrum, generated as a
    continuous stream.

    White complex Gaussian noise is shaped at a low sample rate by a Gaussian FIR filter whose
    power response is ``exp(-f² / (2σ²))`` with ``σ = spread / 2``, then linearly interpolated
    up to ``fs``. Because the process is band-limited to a few hertz, linear interpolation
    from ≥ 100 Hz is far below any measurable error. The filter runs warm from construction,
    so the first sample is already a steady-state realisation.
    """

    def __init__(self, fs: float, doppler_spread_hz: float, rng: np.random.Generator) -> None:
        self.fs = float(fs)
        self.doppler_spread_hz = float(doppler_spread_hz)
        self._rng = rng
        self._static = self.doppler_spread_hz <= 0.0
        if self._static:
            return

        sigma_f = self.doppler_spread_hz / 2.0
        # Low rate: at least 100 Hz and at least 40 σ so the spectrum sits well inside Nyquist.
        fs_low_target = max(100.0, 40.0 * sigma_f)
        self._decim = max(1, math.floor(self.fs / fs_low_target))
        self.fs_low = self.fs / self._decim

        # Time-domain Gaussian whose *power* spectrum is exp(-f²/(2σ²)).
        sigma_t = 1.0 / (2.0 * math.sqrt(2.0) * math.pi * sigma_f)
        half = math.ceil(4.0 * sigma_t * self.fs_low)
        t = np.arange(-half, half + 1) / self.fs_low
        h = np.exp(-(t**2) / (2.0 * sigma_t**2))
        h /= np.sqrt(np.sum(h**2))  # unit-variance input → unit-power output
        self._taps = h
        self._zi = np.zeros(len(h) - 1, dtype=np.complex128)

        # Warm up past the filter's memory so the stream starts in steady state.
        self._lowrate(2 * len(h))
        g = self._lowrate(2)
        self._g0 = complex(g[0])
        self._g1 = complex(g[1])
        self._pos = 0  # offset (in fs samples) of the next output within [g0, g1]

    def _lowrate(self, n: int) -> ComplexArray:
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        w = complex_normal(self._rng, n)
        y, self._zi = signal.lfilter(self._taps, [1.0], w, zi=self._zi)
        return np.asarray(y, dtype=np.complex128)

    def lowrate(self, n: int) -> ComplexArray:
        """Return ``n`` samples of the fading process at ``fs_low`` (for statistical tests)."""
        if self._static:
            return np.ones(n, dtype=np.complex128)
        return self._lowrate(n)

    def next(self, n: int) -> ComplexArray:
        """Return the next ``n`` fading gains at ``fs``."""
        if self._static:
            return np.ones(n, dtype=np.complex128)
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        d = self._decim
        t = self._pos + np.arange(n)
        k = t // d
        frac = (t % d) / d
        t_next = self._pos + n
        k_next = t_next // d
        new = self._lowrate(int(k_next))
        grid = np.concatenate(([self._g0, self._g1], new))
        g = grid[k] + (grid[k + 1] - grid[k]) * frac
        self._g0 = complex(grid[k_next])
        self._g1 = complex(grid[k_next + 1])
        self._pos = int(t_next % d)
        return np.asarray(g, dtype=np.complex128)


# ── Watterson multipath ───────────────────────────────────────────────


class WattersonChannel:
    """Tapped delay line with independent Rayleigh-fading taps and unit total mean power."""

    def __init__(self, profile: ChannelProfile | str, fs: float, seed: int = 0) -> None:
        self.profile = get_profile(profile)
        self.fs = float(fs)
        n_taps = len(self.profile.delays_s)
        seeds = np.random.SeedSequence(seed).spawn(n_taps)
        self._delays = [round(d * self.fs) for d in self.profile.delays_s]
        self._max_delay = max(self._delays)
        powers = 10.0 ** (np.asarray(self.profile.tap_powers_db) / 10.0)
        self._amps = np.sqrt(powers / powers.sum())
        self._fading = [
            RayleighFadingProcess(self.fs, spread, np.random.default_rng(s))
            for spread, s in zip(self.profile.doppler_spread_hz, seeds, strict=True)
        ]
        self._history = np.zeros(self._max_delay, dtype=np.complex128)

    def process(self, x: ComplexArray) -> ComplexArray:
        x = np.asarray(x, dtype=np.complex128)
        n = len(x)
        buf = np.concatenate((self._history, x))
        y = np.zeros(n, dtype=np.complex128)
        for delay, amp, fading in zip(self._delays, self._amps, self._fading, strict=True):
            start = self._max_delay - delay
            y += amp * fading.next(n) * buf[start : start + n]
        if self._max_delay:
            self._history = buf[-self._max_delay :]
        return y


# ── Additive noise ────────────────────────────────────────────────────


def noise_power_for_snr(
    snr_db: float,
    signal_power: float,
    fs: float,
    reference_bandwidth_hz: float = SNR_REFERENCE_BANDWIDTH_HZ,
) -> float:
    """Total complex-noise power over ``fs`` that yields ``snr_db`` in the reference bandwidth."""
    return signal_power * fs / (reference_bandwidth_hz * 10.0 ** (snr_db / 10.0))


class Awgn:
    """White complex Gaussian noise at a 3 kHz-referenced SNR, with optional impulsive bursts.

    Impulsive noise is a simplified Middleton class-A model: each sample is, with probability
    ``impulsive_probability``, hit by an extra Gaussian burst ``impulsive_db_above_noise`` dB
    above the Gaussian floor.
    """

    def __init__(
        self,
        fs: float,
        snr_db: float,
        *,
        seed: int | np.random.SeedSequence = 0,
        impulsive_probability: float = 0.0,
        impulsive_db_above_noise: float = 20.0,
        reference_bandwidth_hz: float = SNR_REFERENCE_BANDWIDTH_HZ,
    ) -> None:
        self.fs = float(fs)
        self.snr_db = float(snr_db)
        ss = seed if isinstance(seed, np.random.SeedSequence) else np.random.SeedSequence(seed)
        gauss, hit, burst = (np.random.default_rng(s) for s in ss.spawn(3))
        self._rng_gauss, self._rng_hit, self._rng_burst = gauss, hit, burst
        self.impulsive_probability = float(impulsive_probability)
        self.impulsive_db_above_noise = float(impulsive_db_above_noise)
        self.reference_bandwidth_hz = float(reference_bandwidth_hz)

    def noise_power(self, signal_power: float) -> float:
        return noise_power_for_snr(self.snr_db, signal_power, self.fs, self.reference_bandwidth_hz)

    def process(self, x: ComplexArray, signal_power: float) -> ComplexArray:
        n = len(x)
        total = self.noise_power(signal_power)
        noise = math.sqrt(total) * complex_normal(self._rng_gauss, n)
        if self.impulsive_probability > 0.0:
            hit = self._rng_hit.random(n) < self.impulsive_probability
            burst_power = total * 10.0 ** (self.impulsive_db_above_noise / 10.0)
            noise = noise + hit * (math.sqrt(burst_power) * complex_normal(self._rng_burst, n))
        return x + noise


# ── Receiver imperfections ────────────────────────────────────────────


class FrequencyOffset:
    """Carrier frequency offset with optional linear drift; phase-continuous across blocks."""

    def __init__(self, fs: float, offset_hz: float, drift_hz_per_s: float = 0.0) -> None:
        self.fs = float(fs)
        self.offset_hz = float(offset_hz)
        self.drift_hz_per_s = float(drift_hz_per_s)
        self._n0 = 0

    def process(self, x: ComplexArray) -> ComplexArray:
        n = len(x)
        t = (self._n0 + np.arange(n)) / self.fs
        self._n0 += n
        phase = 2.0 * np.pi * (self.offset_hz * t + 0.5 * self.drift_hz_per_s * t**2)
        return x * np.exp(1j * phase)


class SampleRateOffset:
    """Resamples by ``1 + ppm·1e-6`` with a streaming cubic (4-point Lagrange) interpolator.

    Positive ppm means the receiver's clock runs *fast* relative to the transmitter's: it
    consumes more than one input sample per output sample, so tones appear shifted up by
    the same ppm and the block comes back slightly shorter.
    """

    def __init__(self, fs: float, ppm: float) -> None:
        self.fs = float(fs)
        self.ppm = float(ppm)
        self._ratio = 1.0 + self.ppm * 1e-6
        self._history = np.zeros(3, dtype=np.complex128)
        self._p = 3.0  # position of the next output sample within [history + block]

    def process(self, x: ComplexArray) -> ComplexArray:
        buf = np.concatenate((self._history, np.asarray(x, dtype=np.complex128)))
        last_ok = len(buf) - 3  # highest integer index i such that i + 2 is inside buf
        m = math.floor((last_ok - self._p) / self._ratio) + 1
        if m <= 0:
            self._history = buf
            return np.zeros(0, dtype=np.complex128)
        p = self._p + self._ratio * np.arange(m)
        i = np.floor(p).astype(np.int64)
        mu = p - i
        w_m1 = -mu * (mu - 1.0) * (mu - 2.0) / 6.0
        w_0 = (mu + 1.0) * (mu - 1.0) * (mu - 2.0) / 2.0
        w_1 = -(mu + 1.0) * mu * (mu - 2.0) / 2.0
        w_2 = (mu + 1.0) * mu * (mu - 1.0) / 6.0
        y = w_m1 * buf[i - 1] + w_0 * buf[i] + w_1 * buf[i + 1] + w_2 * buf[i + 2]
        p_next = self._p + self._ratio * m
        i_next = math.floor(p_next)
        keep_from = max(i_next - 1, 0)
        self._history = buf[keep_from:]
        self._p = p_next - keep_from
        return np.asarray(y, dtype=np.complex128)


# ── Transmitter imperfection ──────────────────────────────────────────


class SaturatingPa:
    """Rapp solid-state PA model applied to the complex envelope.

    ``y = x / (1 + (|x| / A_sat)^(2p))^(1/(2p))``. ``smoothness`` p → ∞ is a hard limiter;
    p ≈ 2 is typical of solid-state HF amplifiers. Construct via :meth:`from_backoff` to
    express the saturation point as an input back-off relative to the signal RMS.
    """

    def __init__(self, saturation_amplitude: float, smoothness: float = 2.0) -> None:
        if saturation_amplitude <= 0:
            raise ValueError("saturation amplitude must be positive")
        self.saturation_amplitude = float(saturation_amplitude)
        self.smoothness = float(smoothness)

    @classmethod
    def from_backoff(
        cls, signal_power: float, input_backoff_db: float, smoothness: float = 2.0
    ) -> SaturatingPa:
        a_sat = math.sqrt(signal_power) * 10.0 ** (input_backoff_db / 20.0)
        return cls(a_sat, smoothness)

    def process(self, x: ComplexArray) -> ComplexArray:
        a = np.abs(x)
        p2 = 2.0 * self.smoothness
        return x / (1.0 + (a / self.saturation_amplitude) ** p2) ** (1.0 / p2)


# ── Interferer ────────────────────────────────────────────────────────

InterfererKind = Literal["cw", "noise"]


class Interferer:
    """An adjacent/co-channel interferer: a CW carrier or a band-limited noise signal (an
    SSB-voice-like occupant) at ``offset_hz`` with power ``power_db`` relative to the signal.
    """

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        *,
        kind: InterfererKind = "cw",
        bandwidth_hz: float = 2400.0,
        rng: np.random.Generator | None = None,
    ) -> None:
        self.fs = float(fs)
        self.offset_hz = float(offset_hz)
        self.power_db = float(power_db)
        self.kind = kind
        self.bandwidth_hz = float(bandwidth_hz)
        self._rng = rng if rng is not None else np.random.default_rng(0)
        self._n0 = 0
        if kind == "noise":
            cutoff = min(0.5 * bandwidth_hz, 0.49 * self.fs)
            self._sos = signal.butter(6, cutoff, fs=self.fs, output="sos")
            self._zi = np.zeros((self._sos.shape[0], 2), dtype=np.complex128)
            impulse = np.zeros(8192)
            impulse[0] = 1.0
            self._gain = float(np.sum(np.abs(signal.sosfilt(self._sos, impulse)) ** 2))
        elif kind != "cw":
            raise ValueError(f"unknown interferer kind {kind!r}")

    def next(self, n: int, signal_power: float) -> ComplexArray:
        power = signal_power * 10.0 ** (self.power_db / 10.0)
        t = (self._n0 + np.arange(n)) / self.fs
        self._n0 += n
        carrier = np.exp(2j * np.pi * self.offset_hz * t).astype(np.complex128)
        if self.kind == "cw":
            return np.asarray(math.sqrt(power) * carrier, dtype=np.complex128)
        w = complex_normal(self._rng, n)
        y, self._zi = signal.sosfilt(self._sos, w, zi=self._zi)
        return np.asarray(
            math.sqrt(power / self._gain) * np.asarray(y) * carrier, dtype=np.complex128
        )


# ── Composite channel ─────────────────────────────────────────────────


@dataclass(frozen=True)
class InterfererConfig:
    offset_hz: float
    power_db: float
    kind: InterfererKind = "cw"
    bandwidth_hz: float = 2400.0


@dataclass(frozen=True)
class ChannelConfig:
    """Everything that defines one simulated link. All impairments default to *off*."""

    profile: ChannelProfile | str = "awgn"
    snr_db: float | None = 20.0
    """SNR in a 3 kHz noise bandwidth; ``None`` disables additive noise."""
    fs: float = 8000.0
    signal_power: float | None = None
    """Reference signal power for SNR/PA/interferer. ``None`` measures each block's mean
    power — fine for continuous benchmarks, wrong for bursts with silence: pass the known
    transmit power then."""
    cfo_hz: float = 0.0
    cfo_drift_hz_per_s: float = 0.0
    sro_ppm: float = 0.0
    impulsive_probability: float = 0.0
    impulsive_db_above_noise: float = 20.0
    interferer: InterfererConfig | None = None
    pa_input_backoff_db: float | None = None
    """Rapp PA saturation point in dB above the signal RMS; ``None`` = perfectly linear."""
    pa_smoothness: float = 2.0
    seed: int = 0


class HfChannel:
    """The full simulated link. See the module docstring for the processing order."""

    def __init__(self, config: ChannelConfig) -> None:
        self.config = config
        fs = config.fs
        seeds = np.random.SeedSequence(config.seed).spawn(3)
        self.fading = WattersonChannel(config.profile, fs, seed=config.seed)
        self.awgn = (
            Awgn(
                fs,
                config.snr_db,
                seed=seeds[0],
                impulsive_probability=config.impulsive_probability,
                impulsive_db_above_noise=config.impulsive_db_above_noise,
            )
            if config.snr_db is not None
            else None
        )
        self.cfo = (
            FrequencyOffset(fs, config.cfo_hz, config.cfo_drift_hz_per_s)
            if config.cfo_hz or config.cfo_drift_hz_per_s
            else None
        )
        self.sro = SampleRateOffset(fs, config.sro_ppm) if config.sro_ppm else None
        ic = config.interferer
        self.interferer = (
            Interferer(
                fs,
                ic.offset_hz,
                ic.power_db,
                kind=ic.kind,
                bandwidth_hz=ic.bandwidth_hz,
                rng=np.random.default_rng(seeds[1]),
            )
            if ic is not None
            else None
        )
        self._pa: SaturatingPa | None = None
        self._pa_backoff = config.pa_input_backoff_db

    @staticmethod
    def _reference_power(x: ComplexArray, configured: float | None) -> float:
        if configured is not None:
            return configured
        p = float(np.mean(np.abs(x) ** 2)) if len(x) else 0.0
        if p <= 0.0:
            raise ValueError(
                "cannot infer signal power from a silent block; set ChannelConfig.signal_power"
            )
        return p

    def process(self, x: ComplexArray) -> ComplexArray:
        x = np.asarray(x, dtype=np.complex128)
        p_ref = self._reference_power(x, self.config.signal_power)
        if self._pa_backoff is not None:
            if self._pa is None:
                self._pa = SaturatingPa.from_backoff(
                    p_ref, self._pa_backoff, self.config.pa_smoothness
                )
            x = self._pa.process(x)
        y = self.fading.process(x)
        if self.interferer is not None:
            y = y + self.interferer.next(len(y), p_ref)
        if self.awgn is not None:
            y = self.awgn.process(y, p_ref)
        if self.cfo is not None:
            y = self.cfo.process(y)
        if self.sro is not None:
            y = self.sro.process(y)
        return y


def make_channel(
    profile: ChannelProfile | str = "awgn",
    snr_db: float | None = 20.0,
    fs: float = 8000.0,
    seed: int = 0,
    **kwargs: object,
) -> HfChannel:
    """Convenience factory: ``make_channel("poor", snr_db=5, cfo_hz=30, sro_ppm=50)``."""
    return HfChannel(ChannelConfig(profile=profile, snr_db=snr_db, fs=fs, seed=seed, **kwargs))  # type: ignore[arg-type]
