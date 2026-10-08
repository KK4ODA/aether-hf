"""Other signals on the band, and the static of distant lightning, for the scenario harness.

The channel simulator (:mod:`aether_model.channel`) models one link: its fading, its noise, a
CW carrier or a band of noise beside it. On 80 and 40 m an Aether session shares the band with
other digital stations and with atmospheric noise, and the field sessions that failed did so
around them (ADR-0042). This module adds them, as the channel's other processes are built:

* **streaming** — each source keeps its state, and splitting a stream into blocks of any size
  gives the same samples (``test_qrm.py`` checks it), so the harness's 20 ms ticks and a
  benchmark's whole frames see one signal;
* **seeded** — a scenario is repeatable from its seed;
* **at complex baseband** — the channel's 8 kHz, centred on the modem's audio centre, so a
  source at audio frequency *f* sits at ``f − 1500`` Hz here;
* **in the channel's units** — a source's power is stated against the modem signal's (unit
  power inside :class:`~aether_model.channel.HfChannel`), a crash's against the noise.

The sources are generic signals of the classes heard on the bands, built from their public
descriptions — occupied bandwidth, keying rate and the rhythm of an ARQ exchange — never from
any program's internals:

* :class:`OfdmArqStation` — a wide (≈ 2.4 kHz) multicarrier ARQ signal of the VARA HF class:
  long data bursts, the other station's short acknowledgements between them, now and then a
  pause;
* :class:`PactorStation` — a PACTOR-class ≈ 500 Hz two-carrier π/4-differential-QPSK ARQ signal on
  the 1.25 s cycle (0.96 s packet, 0.12 s control signal from the other station);
* :class:`RttyStation` — 45.45 Bd, 170 Hz shift FSK, continuous phase, Baudot characters,
  sent in overs of tens of seconds;
* :class:`Ft8Station` — an FT8-class weak-signal station: 8-FSK at 6.25 Bd, 6.25 Hz apart
  (50 Hz occupied), 79 symbols (12.64 s) starting half a second into a 15 s slot, sent in
  some slots and not others — on 20 and 15 m a dozen of them sit beside the data segment;
* :class:`AtmosphericCrashes` — the impulsive noise of distant thunderstorms: crashes arriving
  at random (Poisson), each a cluster of impulses under a decaying envelope tens of
  milliseconds long, its peak a log-normal number of decibels above the noise floor — a
  parametric stand-in for the impulsive atmospheric noise of ITU-R P.372, whose amplitude
  probability distribution it is meant to resemble, not to reproduce.

Each source may fade on its own path (``profile``), as a station heard over the ionosphere
does; the crashes do not.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.channel import WattersonChannel, complex_normal

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

RAMP_S = 0.005
"""Rise and fall of a keyed transmission: a transmitter's own shaping, and no clicks."""


# ── keyed schedules ───────────────────────────────────────────────────


@dataclass
class _Segment:
    start: int
    end: int
    level: float
    """Amplitude while on; 0 for a gap."""


class _Schedule:
    """On/off segments drawn one at a time as the stream reaches them, each with its own
    amplitude, ramped at both ends. The draws happen in stream order whatever the blocking,
    which is what makes a keyed source block-exact."""

    def __init__(self, fs: float, rng: np.random.Generator, pattern: object) -> None:
        self.fs = fs
        self._rng = rng
        self._next_segments = pattern  # a callable returning [(seconds, level), ...]
        self._queue: list[_Segment] = []
        self._cursor = 0
        self._ramp = max(1, round(RAMP_S * fs))

    def _extend(self) -> None:
        for seconds, level in self._next_segments(self._rng):  # type: ignore[operator]
            length = max(1, round(seconds * self.fs))
            self._queue.append(_Segment(self._cursor, self._cursor + length, level))
            self._cursor += length

    def envelope(self, start: int, n: int) -> FloatArray:
        """The amplitude of samples ``start … start + n``."""
        out = np.zeros(n)
        while not self._queue or self._queue[-1].end < start + n:
            self._extend()
        while self._queue and self._queue[0].end <= start:
            self._queue.pop(0)
        for seg in self._queue:
            if seg.start >= start + n:
                break
            if seg.level == 0.0:
                continue
            lo, hi = max(seg.start, start), min(seg.end, start + n)
            k = np.arange(lo, hi)
            ramp = np.minimum(1.0, np.minimum(k - seg.start + 1, seg.end - k) / self._ramp)
            out[lo - start : hi - start] = seg.level * ramp
        return out


# ── the sources ───────────────────────────────────────────────────────


class _Source:
    """A keyed waveform at an offset, optionally through its own fading path."""

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        seed: int,
        profile: str | None,
    ) -> None:
        self.fs = float(fs)
        self.offset_hz = float(offset_hz)
        self.amplitude = 10.0 ** (power_db / 20.0)
        ss = np.random.SeedSequence(seed)
        sched, wave, fade = ss.spawn(3)
        self._rng_wave = np.random.default_rng(wave)
        self._schedule = _Schedule(self.fs, np.random.default_rng(sched), self._pattern)
        self._fading = (
            WattersonChannel(profile, self.fs, seed=int(fade.generate_state(1)[0]))
            if profile is not None
            else None
        )
        self._n = 0

    def _pattern(self, rng: np.random.Generator) -> list[tuple[float, float]]:
        raise NotImplementedError

    def _waveform(self, n: int) -> ComplexArray:
        """``n`` samples of the unit-power waveform at 0 Hz, continuing the last block."""
        raise NotImplementedError

    def next(self, n: int) -> ComplexArray:
        """The next ``n`` samples, in the channel's units."""
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        env = self._schedule.envelope(self._n, n)
        x = self._waveform(n) * env
        t = (self._n + np.arange(n)) / self.fs
        x = x * np.exp(2j * np.pi * self.offset_hz * t)
        self._n += n
        if self._fading is not None:
            x = self._fading.process(x)
        return np.asarray(self.amplitude * x, dtype=np.complex128)


class OfdmArqStation(_Source):
    """A wide multicarrier ARQ signal of the VARA HF class, as an Aether station hears one
    working beside it: its data bursts at ``power_db`` against the modem's signal, the other
    end's acknowledgements at ``partner_db`` more (often less: that station is elsewhere).

    ``carriers`` QPSK carriers ``spacing_hz`` apart (48 × 50 Hz occupies 2.4 kHz), a quarter
    of a symbol of cyclic prefix; data bursts of 3.5–5.5 s, acknowledgements of 0.6–1.2 s,
    0.3–0.6 s of turnaround either side, and one exchange in twenty followed by a pause of
    10–60 s."""

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        *,
        partner_db: float = -6.0,
        carriers: int = 48,
        spacing_hz: float = 50.0,
        seed: int = 0,
        profile: str | None = None,
    ) -> None:
        self.partner = 10.0 ** (partner_db / 20.0)
        self.freqs = (np.arange(carriers) - (carriers - 1) / 2.0) * spacing_hz
        self.useful = round(fs / spacing_hz)
        self.cp = self.useful // 4
        self._symbol: ComplexArray = np.zeros(0, dtype=np.complex128)
        super().__init__(fs, offset_hz, power_db, seed, profile)

    def _pattern(self, rng: np.random.Generator) -> list[tuple[float, float]]:
        out = [
            (rng.uniform(3.5, 5.5), 1.0),
            (rng.uniform(0.3, 0.6), 0.0),
            (rng.uniform(0.6, 1.2), self.partner),
            (rng.uniform(0.3, 0.6), 0.0),
        ]
        if rng.random() < 0.05:
            out.append((rng.uniform(10.0, 60.0), 0.0))
        return out

    def _next_symbol(self) -> ComplexArray:
        phases = self._rng_wave.integers(0, 4, len(self.freqs))
        values = np.exp(1j * (np.pi / 2 * phases + np.pi / 4))
        k = np.arange(-self.cp, self.useful)
        carriers = np.exp(2j * np.pi * self.freqs[None, :] * k[:, None] / self.fs)
        sym = (values[None, :] * carriers).sum(axis=1)
        return np.asarray(sym / math.sqrt(len(self.freqs)), dtype=np.complex128)

    def _waveform(self, n: int) -> ComplexArray:
        while len(self._symbol) < n:
            self._symbol = np.concatenate((self._symbol, self._next_symbol()))
        out, self._symbol = self._symbol[:n], self._symbol[n:]
        return out


class PactorStation(_Source):
    """A PACTOR-class ARQ signal: two carriers 200 Hz apart, π/4 differential QPSK at 100 Bd with
    a raised-cosine phase transition (≈ 500 Hz occupied), on the 1.25 s cycle — a 0.96 s packet,
    0.12 s control signal from the other end at ``partner_db``. One cycle in two hundred is
    followed by a 5–30 s pause (a changeover, a listen)."""

    BAUD = 100.0
    SHIFT_HZ = 100.0
    CYCLE_S = 1.25
    PACKET_S = 0.96
    CONTROL_S = 0.12

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        *,
        partner_db: float = -6.0,
        seed: int = 0,
        profile: str | None = None,
    ) -> None:
        self.partner = 10.0 ** (partner_db / 20.0)
        self.sps = round(fs / self.BAUD)
        self._phase = np.zeros(2)
        self._pending: ComplexArray = np.zeros(0, dtype=np.complex128)
        self._t = 0
        super().__init__(fs, offset_hz, power_db, seed, profile)

    def _pattern(self, rng: np.random.Generator) -> list[tuple[float, float]]:
        gap = (self.CYCLE_S - self.PACKET_S - self.CONTROL_S) / 2.0
        out = [(self.PACKET_S, 1.0), (gap, 0.0), (self.CONTROL_S, self.partner), (gap, 0.0)]
        if rng.random() < 0.005:
            out.append((rng.uniform(5.0, 30.0), 0.0))
        return out

    def _next_symbol(self) -> ComplexArray:
        # π/4-shifted: steps of ±π/4 and ±3π/4, symmetric about the carrier
        steps = (self._rng_wave.integers(0, 4, 2) - 1.5) * (np.pi / 2)
        start = self._phase.copy()
        self._phase = (self._phase + steps) % (2 * np.pi)
        k = np.arange(self.sps)
        # the phase moves to its new value over the first half of the symbol, smoothly
        frac = np.clip(k / (self.sps / 2), 0.0, 1.0)
        shape = 0.5 - 0.5 * np.cos(np.pi * frac)
        t = (self._t + k) / self.fs
        self._t += self.sps
        out = np.zeros(self.sps, dtype=np.complex128)
        for i, f in enumerate((-self.SHIFT_HZ, self.SHIFT_HZ)):
            phase = start[i] + steps[i] * shape
            out += np.exp(1j * (2 * np.pi * f * t + phase))
        return out / math.sqrt(2.0)

    def _waveform(self, n: int) -> ComplexArray:
        while len(self._pending) < n:
            self._pending = np.concatenate((self._pending, self._next_symbol()))
        out, self._pending = self._pending[:n], self._pending[n:]
        return out


class RttyStation(_Source):
    """Amateur RTTY: 45.45 Bd FSK, mark 85 Hz above the offset and space 85 Hz below,
    continuous phase, Baudot characters (a start bit, five data bits, 1.5 stop bits), sent in
    overs of 20–60 s with 5–30 s between them — a contest station, or a bulletin."""

    BAUD = 45.45
    SHIFT_HZ = 170.0

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        *,
        seed: int = 0,
        profile: str | None = None,
    ) -> None:
        self.bit = fs / self.BAUD
        self._phase = 0.0
        self._pending: FloatArray = np.zeros(0)
        self._carry = 0.0
        super().__init__(fs, offset_hz, power_db, seed, profile)

    def _pattern(self, rng: np.random.Generator) -> list[tuple[float, float]]:
        return [(rng.uniform(20.0, 60.0), 1.0), (rng.uniform(5.0, 30.0), 0.0)]

    def _next_character(self) -> FloatArray:
        """The instantaneous frequency of one character, a sample at a time."""
        bits = [0, *self._rng_wave.integers(0, 2, 5).tolist(), 1, 1]
        widths = [1.0] * 7 + [0.5]  # 1.5 stop bits
        out = []
        for b, w in zip(bits, widths, strict=True):
            exact = self.bit * w + self._carry
            count = int(exact)
            self._carry = exact - count
            out.append(np.full(count, self.SHIFT_HZ / 2 if b else -self.SHIFT_HZ / 2))
        return np.concatenate(out)

    def _waveform(self, n: int) -> ComplexArray:
        while len(self._pending) < n:
            self._pending = np.concatenate((self._pending, self._next_character()))
        freq, self._pending = self._pending[:n], self._pending[n:]
        phase = self._phase + 2 * np.pi * np.cumsum(freq) / self.fs
        self._phase = float(phase[-1] % (2 * np.pi)) if n else self._phase
        return np.exp(1j * phase)


class Ft8Station(_Source):
    """FT8 as its public description gives it: 8-tone FSK, 6.25 Hz spacing and symbol rate,
    continuous phase, 79 symbols in 12.64 s, starting 0.5 s after a 15 s boundary. A station
    transmits in ``duty`` of its slots — a QSO is every other one, a CQ caller most — and is
    silent in the rest. The tones are random: what matters to Aether is the energy and its
    rhythm, not the message."""

    BAUD = 6.25
    SPACING_HZ = 6.25
    SYMBOLS = 79
    SLOT_S = 15.0
    START_S = 0.5

    def __init__(
        self,
        fs: float,
        offset_hz: float,
        power_db: float,
        *,
        duty: float = 0.5,
        seed: int = 0,
        profile: str | None = None,
    ) -> None:
        self.duty = duty
        self.symbol = fs / self.BAUD
        self._phase = 0.0
        self._pending: FloatArray = np.zeros(0)
        self._carry = 0.0
        super().__init__(fs, offset_hz, power_db, seed, profile)

    def _pattern(self, rng: np.random.Generator) -> list[tuple[float, float]]:
        on = self.SYMBOLS / self.BAUD
        if rng.random() < self.duty:
            return [(self.START_S, 0.0), (on, 1.0), (self.SLOT_S - self.START_S - on, 0.0)]
        return [(self.SLOT_S, 0.0)]

    def _waveform(self, n: int) -> ComplexArray:
        while len(self._pending) < n:
            exact = self.symbol + self._carry
            count = int(exact)
            self._carry = exact - count
            tone = int(self._rng_wave.integers(0, 8))
            hz = (tone - 3.5) * self.SPACING_HZ
            self._pending = np.concatenate((self._pending, np.full(count, hz)))
        freq, self._pending = self._pending[:n], self._pending[n:]
        phase = self._phase + 2 * np.pi * np.cumsum(freq) / self.fs
        self._phase = float(phase[-1] % (2 * np.pi)) if n else self._phase
        return np.exp(1j * phase)


class AtmosphericCrashes:
    """Static crashes from distant thunderstorms, against the noise floor.

    Crashes arrive at ``rate_per_s`` (Poisson). Each lasts 5–80 ms (log-uniform), is a white
    complex Gaussian burst under an envelope that rises within a millisecond and decays
    exponentially over its length, modulated by a train of sub-impulses (the return strokes),
    and peaks ``N(peak_db, spread_db)`` dB above ``noise_power`` — the floor's power at the
    channel's rate. A rate of 0.2–2 a second and peaks of 15–35 dB are what a summer evening on
    80 m sounds like; the white bursts are band-limited by whatever filter follows."""

    def __init__(
        self,
        fs: float,
        noise_power: float,
        *,
        rate_per_s: float = 0.5,
        peak_db: float = 25.0,
        spread_db: float = 6.0,
        seed: int = 0,
    ) -> None:
        self.fs = float(fs)
        self.noise_power = float(noise_power)
        self.rate = float(rate_per_s)
        self.peak_db = float(peak_db)
        self.spread_db = float(spread_db)
        ss = np.random.SeedSequence(seed)
        arrive, shape, noise = ss.spawn(3)
        self._rng_arrive = np.random.default_rng(arrive)
        self._rng_shape = np.random.default_rng(shape)
        self._rng_noise = np.random.default_rng(noise)
        self._crashes: list[tuple[int, FloatArray]] = []
        self._next_at = self._gap()
        self._n = 0

    def _gap(self) -> int:
        if self.rate <= 0.0:
            return 2**62
        return max(1, round(self._rng_arrive.exponential(1.0 / self.rate) * self.fs))

    def _envelope(self) -> FloatArray:
        length = math.exp(self._rng_shape.uniform(math.log(0.005), math.log(0.08)))
        n = max(8, round(length * self.fs))
        k = np.arange(n)
        rise = np.minimum(1.0, k / max(1.0, 0.001 * self.fs))
        decay = np.exp(-3.0 * k / n)
        strokes = np.ones(n)
        count = int(self._rng_shape.integers(1, 6))
        for at in self._rng_shape.integers(0, n, count):
            width = max(1, round(0.0005 * self.fs))
            strokes[at : at + width] += self._rng_shape.uniform(1.0, 4.0)
        peak_db = self._rng_shape.normal(self.peak_db, self.spread_db)
        amplitude = math.sqrt(self.noise_power * 10.0 ** (peak_db / 10.0))
        env = rise * decay * strokes
        return amplitude * env / max(float(env.max()), 1e-12)

    def next(self, n: int) -> ComplexArray:
        if n <= 0:
            return np.zeros(0, dtype=np.complex128)
        start, end = self._n, self._n + n
        while self._next_at < end:
            self._crashes.append((self._next_at, self._envelope()))
            self._next_at += self._gap()
        env = np.zeros(n)
        keep = []
        for at, shape in self._crashes:
            lo, hi = max(at, start), min(at + len(shape), end)
            if lo < hi:
                env[lo - start : hi - start] += shape[lo - at : hi - at]
            if at + len(shape) > end:
                keep.append((at, shape))
        self._crashes = keep
        self._n = end
        return np.asarray(env * complex_normal(self._rng_noise, n), dtype=np.complex128)
