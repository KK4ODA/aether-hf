"""The FM channel model (ADR-0115, the FM proposal's FM-3): a radio at each end.

Conventions (these are the ones every Aether FM number is quoted in):

* **Audio is real, at 48 kHz**, the sound card's rate. The RF path is complex baseband at
  48 kHz around the carrier.
* **CNR is referenced to a 16 kHz noise bandwidth** (:data:`CNR_REFERENCE_BANDWIDTH_HZ`),
  whatever the receiver's IF filter: Carson's bandwidth of a 25 kHz channel, 2 × (5 kHz
  deviation + 3 kHz audio). Two receivers at the same CNR see the same carrier and the same
  noise density; a narrower IF admits less of the noise. :func:`cnr_db` relates it to the
  power at the antenna.
* **Levels.** The modem's samples are in full-scale units. The transmitter's modulation is in
  units of its rated peak deviation, so 1.0 is where its limiter sits. The receiver's audio is
  in units of its own rated deviation until its output gain, then in full-scale units at the ADC.
* **Every stage is a streaming process.** Feeding a signal in blocks gives the same output, bit
  for bit, as feeding it whole (``test_fm_channel.py`` holds every stage and the link to this).
  Positions that matter (the clocks' resampling, the squelch's decisions, the RF offset's phase)
  are computed from absolute sample counts, never accumulated per block.
* **Fading has unit mean power**, and nothing is normalised per block.
* **Seeds are explicit**: one seed per link, split into one generator per random stage.

The chain, in the order a sample passes::

    modem ─► sound card out ─► radio input ─► voice path or flat port ─► modulator (key)
          ─► mobile fading ─► + noise at the CNR ─► IF filter ─► discriminator ─► squelch
          ─► voice path or flat port ─► + the radio's floor ─► output gain ─► sound card in

The repeater (ADR-0115 §2.3) is built after the simplex chain is calibrated (step 10.2b).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Literal

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.channel import complex_normal

FloatArray = NDArray[np.float64]
ComplexArray = NDArray[np.complex128]

FS = 48_000.0
"""The audio and RF sample rate of the model."""

CNR_REFERENCE_BANDWIDTH_HZ = 16_000.0
"""Noise bandwidth every FM CNR is referenced to (ADR-0115 §2.2)."""

THERMAL_DBM_PER_HZ = -174.0
"""kT at 290 K."""

Port = Literal["voice", "flat"]
"""A radio's audio path: the microphone and speaker path, or the flat (9600) data port."""


def cnr_db(p_dbm: float, noise_figure_db: float = 6.0) -> float:
    """The CNR in 16 kHz of a carrier of ``p_dbm`` at the antenna of a receiver with the given
    noise figure."""
    noise_dbm = THERMAL_DBM_PER_HZ + 10.0 * math.log10(CNR_REFERENCE_BANDWIDTH_HZ) + noise_figure_db
    return p_dbm - noise_dbm


def noise_power_for_cnr(cnr: float, fs: float = FS) -> float:
    """Complex noise power over the whole sample rate that puts a unit carrier at ``cnr`` dB
    in the reference bandwidth."""
    return fs / (CNR_REFERENCE_BANDWIDTH_HZ * 10.0 ** (cnr / 10.0))


# ── Streaming primitives ──────────────────────────────────────────────


class _Sos:
    """IIR second-order sections with their state carried between blocks."""

    def __init__(self, sos: NDArray[np.float64], complex_input: bool = False) -> None:
        self._sos = np.asarray(sos, dtype=np.float64)
        self.sos = self._sos
        dtype = np.complex128 if complex_input else np.float64
        self._zi = np.zeros((self._sos.shape[0], 2), dtype=dtype)

    def process(self, x: NDArray[np.generic]) -> NDArray[np.generic]:
        if len(x) == 0:
            return np.asarray(x).copy()
        y, self._zi = signal.sosfilt(self._sos, x, zi=self._zi)
        return np.asarray(y)


class _Iir:
    """A direct-form IIR filter (``b``, ``a``) with its state carried between blocks."""

    def __init__(self, b: NDArray[np.float64], a: NDArray[np.float64]) -> None:
        self._b = np.asarray(b, dtype=np.float64)
        self._a = np.asarray(a, dtype=np.float64)
        self._zi = np.zeros(max(len(self._a), len(self._b)) - 1, dtype=np.float64)

    def process(self, x: FloatArray) -> FloatArray:
        if len(x) == 0:
            return x.copy()
        y, self._zi = signal.lfilter(self._b, self._a, x, zi=self._zi)
        return np.asarray(y, dtype=np.float64)


def _butter(order: int, hz: float, kind: Literal["lowpass", "highpass"]) -> NDArray[np.float64]:
    return np.asarray(signal.butter(order, hz, kind, fs=FS, output="sos"), dtype=np.float64)


def emphasis(tau_us: float, pole_hz: float = 6_000.0) -> tuple[FloatArray, FloatArray]:
    """Pre-emphasis ``(b, a)`` of time constant ``tau_us``: a zero at 1/(2π·τ) (212 Hz for
    750 µs) and a pole at ``pole_hz`` to keep it bounded, unity at 1 kHz."""
    fz = 1.0 / (2.0 * math.pi * tau_us * 1e-6)
    b, a = signal.bilinear(
        [1.0 / (2.0 * math.pi * fz), 1.0], [1.0 / (2.0 * math.pi * pole_hz), 1.0], FS
    )
    _, h = signal.freqz(b, a, [1000.0], fs=FS)
    return np.asarray(b, dtype=np.float64) / abs(h[0]), np.asarray(a, dtype=np.float64)


def deemphasis(tau_us: float, pole_hz: float = 6_000.0) -> tuple[FloatArray, FloatArray]:
    """The inverse of :func:`emphasis`, unity at 1 kHz."""
    b, a = emphasis(tau_us, pole_hz)
    return a, b


class Clock:
    """A sound card's clock ``ppm`` off the reference, as a streaming 4-point Lagrange
    resampler.

    ``role="dac"``: a fast DAC plays more samples a second, so it consumes ``1 + ppm·1e-6``
    input samples per reference output sample and a tone comes out higher. ``role="adc"``: a
    fast ADC takes more samples a second, so a reference tone reads lower in its samples.
    Output ``k`` sits at input position ``k·ratio``, computed from ``k`` itself, so blocking
    never moves it. The last two input samples are held until the next block arrives.
    """

    def __init__(self, ppm: float, role: Literal["dac", "adc"]) -> None:
        self.ppm = float(ppm)
        r = 1.0 + self.ppm * 1e-6
        self.ratio = r if role == "dac" else 1.0 / r
        self._identity = self.ppm == 0.0
        self._buf = np.zeros(1, dtype=np.float64)  # the virtual sample at index -1 is 0
        self._base = -1  # absolute input index of _buf[0]
        self._k = 0  # absolute index of the next output sample

    def process(self, x: FloatArray) -> FloatArray:
        if self._identity:
            return np.asarray(x, dtype=np.float64).copy()
        self._buf = np.concatenate((self._buf, np.asarray(x, dtype=np.float64)))
        last = self._base + len(self._buf) - 1  # highest absolute input index held
        # output k needs floor(k·ratio) + 2 ≤ last
        k_end = max(self._k, math.ceil((last - 1) / self.ratio) + 1)
        ks = np.arange(self._k, k_end, dtype=np.int64)
        p = ks.astype(np.float64) * self.ratio
        i = np.floor(p).astype(np.int64)
        ok = i + 2 <= last
        ks, p, i = ks[ok], p[ok], i[ok]
        if len(ks) == 0:
            return np.zeros(0, dtype=np.float64)
        mu = p - i
        j = i - self._base
        b = self._buf
        w_m1 = -mu * (mu - 1.0) * (mu - 2.0) / 6.0
        w_0 = (mu + 1.0) * (mu - 1.0) * (mu - 2.0) / 2.0
        w_1 = -(mu + 1.0) * mu * (mu - 2.0) / 2.0
        w_2 = (mu + 1.0) * mu * (mu - 1.0) / 6.0
        y = w_m1 * b[j - 1] + w_0 * b[j] + w_1 * b[j + 1] + w_2 * b[j + 2]
        self._k = int(ks[-1]) + 1
        keep_from = math.floor(self._k * self.ratio) - 1  # absolute index the next output needs
        drop = keep_from - self._base
        if drop > 0:
            self._buf = self._buf[drop:]
            self._base = keep_from
        return np.asarray(y, dtype=np.float64)


# ── Sound cards ───────────────────────────────────────────────────────


@dataclass(frozen=True)
class SoundCardConfig:
    """A sound card: the operating system's volume, clipping at full scale, quantisation, its
    clock, and its own noise."""

    volume_db: float = 0.0
    bits: int | None = 16
    clock_ppm: float = 0.0
    noise_dbfs: float | None = None


def _quantise(x: FloatArray, bits: int | None) -> FloatArray:
    y = np.clip(x, -1.0, 1.0)
    if bits is None:
        return y
    q = float(2 ** (bits - 1))
    return np.asarray(np.round(y * q) / q, dtype=np.float64)


class SoundCardOut:
    """The modem's samples to the analog line: volume, full-scale clipping, quantisation, the
    DAC's clock, then its noise."""

    def __init__(self, config: SoundCardConfig, rng: np.random.Generator) -> None:
        self.config = config
        self._gain = 10.0 ** (config.volume_db / 20.0)
        self._clock = Clock(config.clock_ppm, "dac")
        self._rng = rng

    def process(self, x: FloatArray) -> FloatArray:
        y = self._clock.process(
            _quantise(np.asarray(x, dtype=np.float64) * self._gain, self.config.bits)
        )
        if self.config.noise_dbfs is not None:
            y = y + 10.0 ** (self.config.noise_dbfs / 20.0) * self._rng.standard_normal(len(y))
        return y


class SoundCardIn:
    """The analog line to the modem's samples: the ADC's clock, its noise, the input gain,
    full-scale clipping and quantisation."""

    def __init__(self, config: SoundCardConfig, rng: np.random.Generator) -> None:
        self.config = config
        self._gain = 10.0 ** (config.volume_db / 20.0)
        self._clock = Clock(config.clock_ppm, "adc")
        self._rng = rng

    def process(self, x: FloatArray) -> FloatArray:
        y = self._clock.process(np.asarray(x, dtype=np.float64))
        if self.config.noise_dbfs is not None:
            y = y + 10.0 ** (self.config.noise_dbfs / 20.0) * self._rng.standard_normal(len(y))
        return _quantise(y * self._gain, self.config.bits)


# ── The transmitter ───────────────────────────────────────────────────


@dataclass(frozen=True)
class Compressor:
    """A microphone ALC or compressor ahead of the pre-emphasis (the TM-V71A has one).

    A peak envelope with separate attack and release; above ``threshold`` (in limiter units at
    its input) the gain falls so the output rises ``1/ratio`` dB per dB."""

    threshold: float = 0.25
    ratio: float = 4.0
    attack_s: float = 0.005
    release_s: float = 0.25


class _CompressorState:
    def __init__(self, c: Compressor) -> None:
        self._c = c
        self._att = 1.0 - math.exp(-1.0 / (c.attack_s * FS))
        self._rel = 1.0 - math.exp(-1.0 / (c.release_s * FS))
        self._env = 0.0
        self._slope = 1.0 - 1.0 / c.ratio

    def process(self, x: FloatArray) -> FloatArray:
        env = self._env
        att, rel, thr, slope = self._att, self._rel, self._c.threshold, self._slope
        mag = np.abs(x)
        g = np.empty(len(x), dtype=np.float64)
        for n in range(len(x)):
            m = float(mag[n])
            env += (att if m > env else rel) * (m - env)
            g[n] = 1.0 if env <= thr else (thr / env) ** slope
        self._env = env
        return np.asarray(x * g, dtype=np.float64)


@dataclass(frozen=True)
class TransmitterConfig:
    """A radio's sending side. The voice-path stages (``highpass_hz`` to ``splatter_hz``)
    apply only to ``port="voice"``; the flat port has a DC block and ``port_lowpass_hz``."""

    port: Port = "voice"
    gain_db: float = 0.0  # where full scale sits against the limiter (0: full scale is the limiter)
    highpass_hz: float | None = 300.0
    compressor: Compressor | None = None
    emphasis_us: float | None = 750.0
    limiter_smoothness: float | None = None  # None: hard; else the Rapp knee's p
    splatter_hz: float | None = 3_000.0
    port_lowpass_hz: float | None = None
    ctcss_hz: float | None = None
    ctcss_deviation: float = 0.15
    deviation_hz: float = 5_000.0
    rf_offset_hz: float = 0.0
    key_delay_s: float = 0.05
    attack_s: float = 0.01
    release_s: float = 0.005


class Transmitter:
    """Audio from the sound card to a complex RF signal (unit carrier when fully keyed)."""

    def __init__(self, config: TransmitterConfig) -> None:
        self.config = config
        c = config
        self._gain = 10.0 ** (c.gain_db / 20.0)
        voice = c.port == "voice"
        self._highpass = (
            _Sos(_butter(3, c.highpass_hz, "highpass"))
            if voice and c.highpass_hz is not None
            else None
        )
        self._compressor = (
            _CompressorState(c.compressor) if voice and c.compressor is not None else None
        )
        self._pre = _Iir(*emphasis(c.emphasis_us)) if voice and c.emphasis_us is not None else None
        self._dc_block = None if voice else _Sos(_butter(2, 10.0, "highpass"))
        lowpass = c.splatter_hz if voice else c.port_lowpass_hz
        self._lowpass = _Sos(_butter(4, lowpass, "lowpass")) if lowpass is not None else None
        self._integrator = _Iir(np.array([1.0]), np.array([1.0, -1.0]))
        self._n = 0  # absolute sample count, for the CTCSS tone and the RF offset
        self._keyed = False
        self._since = 10**12  # samples since the last key change
        self._env_at_release = 0.0

    def modulation(self, x: FloatArray) -> FloatArray:
        """The modulating signal in units of the rated peak deviation."""
        s = np.asarray(x, dtype=np.float64) * self._gain
        if self._highpass is not None:
            s = np.asarray(self._highpass.process(s), dtype=np.float64)
        if self._compressor is not None:
            s = self._compressor.process(s)
        if self._pre is not None:
            s = self._pre.process(s)
        if self.config.port == "voice":
            p = self.config.limiter_smoothness
            if p is None:
                s = np.clip(s, -1.0, 1.0)
            else:
                s = s / (1.0 + np.abs(s) ** (2.0 * p)) ** (1.0 / (2.0 * p))
        if self._dc_block is not None:
            s = np.asarray(self._dc_block.process(s), dtype=np.float64)
        if self._lowpass is not None:
            s = np.asarray(self._lowpass.process(s), dtype=np.float64)
        if self.config.ctcss_hz is not None:
            t = (self._n + np.arange(len(s), dtype=np.float64)) / FS
            s = s + self.config.ctcss_deviation * np.sin(2.0 * math.pi * self.config.ctcss_hz * t)
        return s

    def _envelope(self, n: int, keyed: bool) -> FloatArray:
        c = self.config
        if keyed != self._keyed:
            if not keyed:
                self._env_at_release = float(self._envelope_at(self._since))
            self._keyed = keyed
            self._since = 0
        since = self._since + np.arange(n, dtype=np.float64)
        self._since += n
        if keyed:
            return self._envelope_at(since)
        if c.release_s <= 0.0:
            return np.zeros(n, dtype=np.float64)
        frac = np.clip(since / (c.release_s * FS), 0.0, 1.0)
        return np.asarray(self._env_at_release * 0.5 * (1.0 + np.cos(np.pi * frac)))

    def _envelope_at(self, since: FloatArray | float) -> FloatArray:
        c = self.config
        t = np.asarray(since, dtype=np.float64) - c.key_delay_s * FS
        if c.attack_s <= 0.0:
            return np.asarray((t >= 0.0).astype(np.float64))
        frac = np.clip(t / (c.attack_s * FS), 0.0, 1.0)
        return np.asarray(0.5 * (1.0 - np.cos(np.pi * frac)))

    def process(self, x: FloatArray, keyed: bool = True) -> ComplexArray:
        """The RF signal for one block of audio, the key held as given for the whole block."""
        m = self.modulation(x)
        n = len(m)
        phase = self._integrator.process(2.0 * math.pi * self.config.deviation_hz / FS * m)
        if self.config.rf_offset_hz != 0.0:
            idx = (self._n + np.arange(n, dtype=np.int64)).astype(np.float64)
            phase = phase + 2.0 * math.pi * self.config.rf_offset_hz * (idx / FS)
        self._n += n
        env = self._envelope(n, keyed)
        return np.asarray(env * np.exp(1j * phase), dtype=np.complex128)


# ── The path ──────────────────────────────────────────────────────────


class ClarkeFading:
    """Unit-power Rayleigh fading with Clarke's (Jakes') Doppler spectrum, as a stream.

    Complex white noise at a low rate (at least 32 times the Doppler) is shaped by an FIR whose
    power response is 1/√(1 − (f/f_d)²) inside ±f_d, then linearly interpolated up to ``fs``,
    as :class:`aether_model.channel.RayleighFadingProcess` does for HF's Gaussian spectrum.
    """

    def __init__(self, doppler_hz: float, rng: np.random.Generator, fs: float = FS) -> None:
        if doppler_hz <= 0.0:
            raise ValueError("a fading path needs a Doppler above 0 Hz")
        self.doppler_hz = float(doppler_hz)
        self._rng = rng
        target = max(200.0, 32.0 * self.doppler_hz)
        self._decim = max(1, math.floor(fs / target))
        self.fs_low = fs / self._decim
        m = 4096
        f = np.fft.fftfreq(m, d=1.0 / self.fs_low)
        u = np.clip(np.abs(f) / self.doppler_hz, 0.0, 0.995)
        psd = np.where(np.abs(f) < self.doppler_hz, 1.0 / np.sqrt(1.0 - u**2), 0.0)
        h = np.fft.fftshift(np.real(np.fft.ifft(np.sqrt(psd))))
        half = min(m // 2 - 1, math.ceil(8.0 * self.fs_low / self.doppler_hz))
        h = h[m // 2 - half : m // 2 + half + 1] * np.hanning(2 * half + 1)
        self._taps = h / np.sqrt(np.sum(h**2))
        self._zi = np.zeros(len(self._taps) - 1, dtype=np.complex128)
        self._lowrate(2 * len(self._taps))
        g = self._lowrate(2)
        self._g0, self._g1 = complex(g[0]), complex(g[1])
        self._pos = 0

    def _lowrate(self, n: int) -> ComplexArray:
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        y, self._zi = signal.lfilter(self._taps, [1.0], complex_normal(self._rng, n), zi=self._zi)
        return np.asarray(y, dtype=np.complex128)

    def lowrate(self, n: int) -> ComplexArray:
        """``n`` samples at ``fs_low``, for statistical tests."""
        return self._lowrate(n)

    def next(self, n: int) -> ComplexArray:
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        d = self._decim
        t = self._pos + np.arange(n)
        k = t // d
        frac = (t % d) / d
        t_next = self._pos + n
        k_next = t_next // d
        grid = np.concatenate(([self._g0, self._g1], self._lowrate(int(k_next))))
        g = grid[k] + (grid[k + 1] - grid[k]) * frac
        self._g0, self._g1 = complex(grid[k_next]), complex(grid[k_next + 1])
        self._pos = int(t_next % d)
        return np.asarray(g, dtype=np.complex128)


@dataclass(frozen=True)
class PathConfig:
    """The RF path: noise at ``cnr_db`` in 16 kHz, and mobile fading if any.

    ``rician_k_db`` None with ``fading="rician"`` is Rayleigh (no line of sight)."""

    cnr_db: float = 30.0
    fading: Literal["none", "rician"] = "none"
    doppler_hz: float = 0.0
    rician_k_db: float | None = None


class Path:
    def __init__(
        self, config: PathConfig, noise_rng: np.random.Generator, fading_rng: np.random.Generator
    ) -> None:
        self.config = config
        self.noise_power = noise_power_for_cnr(config.cnr_db)
        self._noise_rng = noise_rng
        self._fading: ClarkeFading | None = None
        self._los = 0.0
        self._scatter = 1.0
        if config.fading == "rician":
            self._fading = ClarkeFading(config.doppler_hz, fading_rng)
            if config.rician_k_db is not None:
                k = 10.0 ** (config.rician_k_db / 10.0)
                self._los = math.sqrt(k / (k + 1.0))
                self._scatter = math.sqrt(1.0 / (k + 1.0))

    def process(self, s: ComplexArray) -> ComplexArray:
        y = np.asarray(s, dtype=np.complex128)
        if self._fading is not None:
            y = y * (self._los + self._scatter * self._fading.next(len(y)))
        return np.asarray(
            y + math.sqrt(self.noise_power) * complex_normal(self._noise_rng, len(y)),
            dtype=np.complex128,
        )


# ── The receiver ──────────────────────────────────────────────────────


@dataclass(frozen=True)
class SquelchConfig:
    """A carrier squelch: open once the IF power has stood ``threshold_db`` over the receiver's
    noise for ``open_s``; closed ``tail_s`` after it falls. Decided every millisecond."""

    threshold_db: float = 6.0
    open_s: float = 0.08
    tail_s: float = 0.15
    smoothing_s: float = 0.005


SQUELCH_STEP = 48
"""Samples between the squelch's decisions (1 ms), counted from the stream's first sample."""


@dataclass(frozen=True)
class ReceiverConfig:
    """A radio's receiving side. ``highpass_hz``, ``lowpass_hz`` and ``emphasis_us`` are the
    voice path's; ``port_lowpass_hz`` the flat port's. ``floor_db`` is the radio's own audio
    floor under a full-deviation 1 kHz tone; ``hum_db`` adds 60 Hz and four harmonics each at
    that level. ``squelch`` None leaves the audio always on, as many data ports do."""

    port: Port = "voice"
    if_half_hz: float = 7_500.0
    deviation_hz: float = 5_000.0
    emphasis_us: float | None = 750.0
    highpass_hz: float | None = 300.0
    lowpass_hz: float | None = 3_000.0
    port_lowpass_hz: float | None = 6_000.0
    floor_db: float | None = -42.0
    hum_db: float | None = None
    squelch: SquelchConfig | None = field(default_factory=SquelchConfig)
    gain_db: float = 0.0


def _noise_bandwidth(sos: NDArray[np.float64], fs: float, two_sided: bool) -> float:
    w, h = signal.sosfreqz(sos, worN=16384, fs=fs)
    one = float(np.trapezoid(np.abs(h) ** 2, w))
    return 2.0 * one if two_sided else one


class Receiver:
    """A complex RF signal to the radio's audio output (full-scale units at its output)."""

    def __init__(
        self, config: ReceiverConfig, noise_power: float, rng: np.random.Generator
    ) -> None:
        self.config = config
        c = config
        if_sos = _butter(6, c.if_half_hz, "lowpass")
        self._if = _Sos(if_sos, complex_input=True)
        self.if_noise_bandwidth_hz = _noise_bandwidth(if_sos, FS, two_sided=True)
        self._last = complex(1.0, 0.0)
        audio: list[_Sos | _Iir] = []
        floor: list[_Sos] = []
        if c.port == "voice":
            if c.emphasis_us is not None:
                audio.append(_Iir(*deemphasis(c.emphasis_us)))
            for hz, order, kind in (
                (c.highpass_hz, 6, "highpass"),
                (c.lowpass_hz, 4, "lowpass"),
            ):
                if hz is not None:
                    sos = _butter(order, hz, kind)  # type: ignore[arg-type]
                    audio.append(_Sos(sos))
                    floor.append(_Sos(sos))
        else:
            audio.append(_Sos(_butter(2, 10.0, "highpass")))
            floor.append(_Sos(_butter(2, 10.0, "highpass")))
            if c.port_lowpass_hz is not None:
                audio.append(_Sos(_butter(4, c.port_lowpass_hz, "lowpass")))
                floor.append(_Sos(_butter(4, c.port_lowpass_hz, "lowpass")))
        self._audio = audio
        self._floor_filters = floor
        self._rng = rng
        self._floor_gain = 0.0
        if c.floor_db is not None:
            # unit white noise through the floor's filters has this RMS
            rms = 1.0
            if floor:
                w, h = signal.sosfreqz(np.vstack([s.sos for s in floor]), worN=16384, fs=FS)
                rms = math.sqrt(2.0 / FS * float(np.trapezoid(np.abs(h) ** 2, w)))
            # floor_db under a full-deviation 1 kHz tone, whose RMS is 1/√2
            self._floor_gain = 10.0 ** (c.floor_db / 20.0) / math.sqrt(2.0) / rms
        self._out = 10.0 ** (c.gain_db / 20.0)
        self._n = 0
        # squelch
        self._sq = c.squelch
        self.noise_if_power = noise_power * self.if_noise_bandwidth_hz / FS
        if self._sq is not None:
            a = 1.0 - math.exp(-1.0 / (self._sq.smoothing_s * FS))
            self._rssi = _Iir(np.array([a]), np.array([1.0, -(1.0 - a)]))
            self._open_level = self.noise_if_power * 10.0 ** (self._sq.threshold_db / 10.0)
            self._open = False
            self._above = 0  # consecutive decisions above / below the threshold
            self._below = 0

    def _squelch_gate(self, y: ComplexArray) -> FloatArray:
        sq = self._sq
        n = len(y)
        if sq is None:
            return np.ones(n, dtype=np.float64)
        rssi = self._rssi.process(np.abs(y) ** 2)
        gate = np.empty(n, dtype=np.float64)
        open_after = max(1, round(sq.open_s * FS / SQUELCH_STEP))
        close_after = max(1, round(sq.tail_s * FS / SQUELCH_STEP))
        first = (-self._n) % SQUELCH_STEP  # offset of the first decision in this block
        start = 0
        for d in range(first, n, SQUELCH_STEP):
            gate[start:d] = 1.0 if self._open else 0.0
            if rssi[d] > self._open_level:
                self._above += 1
                self._below = 0
            else:
                self._below += 1
                self._above = 0
            if not self._open and self._above >= open_after:
                self._open = True
            elif self._open and self._below >= close_after:
                self._open = False
            start = d
        gate[start:] = 1.0 if self._open else 0.0
        return gate

    def process(self, rf: ComplexArray) -> FloatArray:
        y = np.asarray(self._if.process(np.asarray(rf, dtype=np.complex128)), dtype=np.complex128)
        n = len(y)
        prev = np.concatenate(([self._last], y[:-1])) if n else y
        if n:
            self._last = complex(y[-1])
        d = np.angle(y * np.conj(prev)) * FS / (2.0 * math.pi * self.config.deviation_hz)
        a = np.asarray(d, dtype=np.float64)
        for st in self._audio:
            a = np.asarray(st.process(a), dtype=np.float64)
        if self.config.floor_db is not None:
            f = self._rng.standard_normal(n)
            for fs_ in self._floor_filters:
                f = np.asarray(fs_.process(f), dtype=np.float64)
            a = a + self._floor_gain * f
        if self.config.hum_db is not None:
            t = (self._n + np.arange(n, dtype=np.float64)) / FS
            hum = sum(np.sin(2.0 * math.pi * 60.0 * k * t) for k in range(1, 6))
            a = a + 10.0 ** (self.config.hum_db / 20.0) / math.sqrt(2.0) * np.asarray(hum)
        gate = self._squelch_gate(y)
        self._n += n
        return np.asarray(a * gate * self._out, dtype=np.float64)

    @property
    def squelch_open(self) -> bool:
        return bool(self._sq is not None and self._open) or self._sq is None


# ── The link ──────────────────────────────────────────────────────────


@dataclass(frozen=True)
class FmLinkConfig:
    """One direction of an FM link: the sender's sound card and radio, the path, the receiver's
    radio and sound card."""

    sender_card: SoundCardConfig = field(default_factory=SoundCardConfig)
    transmitter: TransmitterConfig = field(default_factory=TransmitterConfig)
    path: PathConfig = field(default_factory=PathConfig)
    receiver: ReceiverConfig = field(default_factory=ReceiverConfig)
    receiver_card: SoundCardConfig = field(default_factory=SoundCardConfig)
    seed: int = 0


class FmLink:
    """The modem's audio at one station to the modem's audio at the other."""

    def __init__(self, config: FmLinkConfig) -> None:
        self.config = config
        noise, fading, floor, out_noise, in_noise = (
            np.random.default_rng(s) for s in np.random.SeedSequence(config.seed).spawn(5)
        )
        self.sender_card = SoundCardOut(config.sender_card, out_noise)
        self.transmitter = Transmitter(config.transmitter)
        self.path = Path(config.path, noise, fading)
        self.receiver = Receiver(config.receiver, self.path.noise_power, floor)
        self.receiver_card = SoundCardIn(config.receiver_card, in_noise)

    def process(self, x: FloatArray, keyed: bool = True) -> FloatArray:
        """One block of the modem's audio, with the key held as given for the block."""
        a = self.sender_card.process(np.asarray(x, dtype=np.float64))
        rf = self.path.process(self.transmitter.process(a, keyed))
        return self.receiver_card.process(self.receiver.process(rf))


# ── Presets (uncalibrated: ADR-0115 §5) ───────────────────────────────


def wide_voice(cnr: float = 30.0, seed: int = 0) -> FmLinkConfig:
    """A 25 kHz channel, ±5 kHz, the voice path at both ends with 750 µs emphasis."""
    return FmLinkConfig(path=PathConfig(cnr_db=cnr), seed=seed)


def narrow_voice(cnr: float = 30.0, seed: int = 0) -> FmLinkConfig:
    """A 12.5 kHz channel, ±2.5 kHz, the voice path at both ends."""
    return FmLinkConfig(
        transmitter=TransmitterConfig(deviation_hz=2_500.0),
        path=PathConfig(cnr_db=cnr),
        receiver=ReceiverConfig(if_half_hz=4_250.0, deviation_hz=2_500.0),
        seed=seed,
    )


def flat_port(cnr: float = 30.0, seed: int = 0) -> FmLinkConfig:
    """The flat (9600) data port at both ends of a 25 kHz channel: ±3 kHz for a full-scale
    unit, no emphasis, no limiter, no squelch."""
    return FmLinkConfig(
        transmitter=TransmitterConfig(port="flat", deviation_hz=3_000.0),
        path=PathConfig(cnr_db=cnr),
        receiver=ReceiverConfig(port="flat", deviation_hz=3_000.0, squelch=None),
        seed=seed,
    )
