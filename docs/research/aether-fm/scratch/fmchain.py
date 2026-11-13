"""Scratch study for the Aether FM proposal (not part of the repository).

Each candidate waveform goes through a simulated FM voice radio and the information that
survives is measured as a function of drive level and RF carrier-to-noise ratio.

Transmit (mic path): TX high-pass 300 Hz -> 750 us pre-emphasis -> hard limiter at the peak
deviation -> 3 kHz splatter filter -> FM modulator.  Transmit (flat data port): straight to
the modulator, no emphasis, no limiter (over-deviation shows up as IF-filter distortion).
Channel: complex AWGN at a given CNR in the IF filter's noise bandwidth -> IF filter ->
discriminator (so the FM threshold and click noise arise by themselves).
Receive (mic/speaker): de-emphasis -> 300 Hz CTCSS high-pass (6th order) -> 3 kHz low-pass,
plus the radio's own audio hum/noise floor. Receive (data port): DC block, 6 kHz low-pass.

Measure: magnitude-squared coherence between the clean modem output and the received audio,
SNDR(f) = C/(1-C), and the Gaussian-input rate integral over the signal's band
(distortion treated as noise: what a linear-equalising receiver can exploit).
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import scipy.signal as sg

FS = 48_000


# ---------------------------------------------------------------- waveforms


def qam(M: int, n: int, rng: np.random.Generator) -> np.ndarray:
    k = int(np.sqrt(M))
    lv = np.arange(k) * 2 - (k - 1)
    s = rng.choice(lv, n) + 1j * rng.choice(lv, n)
    return s / np.sqrt(np.mean(np.abs(s) ** 2))


def rrc(alpha: float, sps: int, span: int) -> np.ndarray:
    t = np.arange(-span * sps, span * sps + 1) / sps
    h = np.zeros_like(t)
    for i, ti in enumerate(t):
        if abs(ti) < 1e-9:
            h[i] = 1 - alpha + 4 * alpha / np.pi
        elif alpha > 0 and abs(abs(4 * alpha * ti) - 1) < 1e-9:
            h[i] = (alpha / np.sqrt(2)) * (
                (1 + 2 / np.pi) * np.sin(np.pi / (4 * alpha))
                + (1 - 2 / np.pi) * np.cos(np.pi / (4 * alpha))
            )
        else:
            h[i] = (
                np.sin(np.pi * ti * (1 - alpha)) + 4 * alpha * ti * np.cos(np.pi * ti * (1 + alpha))
            ) / (np.pi * ti * (1 - (4 * alpha * ti) ** 2))
    return h / np.sqrt(np.sum(h**2))


def sc(rs: int, fc: float, alpha: float, M: int, dur: float, rng) -> np.ndarray:
    sps = FS // rs
    n = int(dur * rs)
    sym = qam(M, n, rng)
    up = np.zeros(n * sps, complex)
    up[::sps] = sym
    bb = np.convolve(up, rrc(alpha, sps, 10), mode="same")
    t = np.arange(len(bb)) / FS
    x = np.real(bb * np.exp(2j * np.pi * fc * t))
    return x / np.sqrt(np.mean(x**2))


def ofdm(f_lo: float, f_hi: float, df: float, cp_s: float, M: int, dur: float, rng, dfts=False,
         ) -> np.ndarray:
    N = int(FS / df)
    k0, k1 = int(np.ceil(f_lo / df)), int(np.floor(f_hi / df))
    ks = np.arange(k0, k1 + 1)
    cp = int(cp_s * FS)
    out = []
    nsym = int(dur / ((N + cp) / FS))
    for _ in range(nsym):
        d = qam(M, len(ks), rng)
        if dfts:
            d = np.fft.fft(d) / np.sqrt(len(ks))
        X = np.zeros(N, complex)
        X[ks] = d
        X[-ks] = np.conj(d)
        x = np.fft.ifft(X).real
        out.append(np.concatenate([x[-cp:], x]))
    x = np.concatenate(out)
    return x / np.sqrt(np.mean(x**2))


def aether_hf(dur: float, rng, papr_reduction: bool) -> np.ndarray:
    sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "model"))
    from aether_model.frame.codec import FrameCodec
    from aether_model.frame.modes import LONG, MODES
    from aether_model.phy.preamble import FrameHeader, FrameType
    from aether_model.phy.tx import FrameTransmitter
    from aether_model.waveform import WIDE_2300

    tx = FrameTransmitter(WIDE_2300, papr_reduction=papr_reduction)
    mode = 8  # a 16-QAM mode of the wide table
    codec = FrameCodec(MODES[mode], LONG)
    parts = []
    total = 0
    while total < dur * FS:
        payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
        bb = tx.baseband(FrameHeader(FrameType.DATA, mode), LONG, codec.encode(payload))
        a = np.asarray(tx.audio(bb), float)
        a = a[np.abs(a) > 1e-6 * np.max(np.abs(a))]  # drop the converter's silent pad
        parts.append(a)
        total += len(a)
    x = np.concatenate(parts)[: int(dur * FS)]
    return x / np.sqrt(np.mean(x**2))


def cpfsk4(rs: int, f_c: float, h_hz: float, dur: float, rng) -> np.ndarray:
    """4-level continuous-phase FSK in the audio band (constant envelope)."""
    sps = FS // rs
    lv = rng.choice([-3, -1, 1, 3], int(dur * rs))
    f = f_c + np.repeat(lv, sps) * h_hz
    ph = 2 * np.pi * np.cumsum(f) / FS
    x = np.cos(ph)
    return x / np.sqrt(np.mean(x**2))


def papr_db(x: np.ndarray, p: float) -> float:
    """Level over RMS exceeded by a fraction p of samples (real signal)."""
    a = np.sort(np.abs(x) / np.sqrt(np.mean(x**2)))
    return 20 * np.log10(a[int((1 - p) * len(a))])


# ---------------------------------------------------------------- the radio


def bilinear_shelf(fz: float, fp: float) -> tuple[np.ndarray, np.ndarray]:
    """First-order (1 + s/wz)/(1 + s/wp), unity at 1 kHz."""
    b, a = sg.bilinear([1 / (2 * np.pi * fz), 1], [1 / (2 * np.pi * fp), 1], FS)
    w, h = sg.freqz(b, a, [1000], fs=FS)
    return b / abs(h[0]), a


PRE = bilinear_shelf(212.0, 6000.0)  # 750 us
DE = (PRE[1], PRE[0])
TX_HP = sg.butter(3, 300, "highpass", fs=FS, output="sos")
SPLATTER = sg.butter(4, 3000, "lowpass", fs=FS, output="sos")
RX_HP = sg.butter(6, 300, "highpass", fs=FS, output="sos")
RX_LP = sg.butter(4, 3000, "lowpass", fs=FS, output="sos")
DC = sg.butter(2, 20, "highpass", fs=FS, output="sos")
DATA_LP = sg.butter(4, 6000, "lowpass", fs=FS, output="sos")


@dataclass
class Radio:
    path: str  # "mic" or "data"
    dev_hz: float  # peak deviation that the limiter (or 1.0 units) stands for
    if_half_hz: float  # IF filter cut-off, each side
    audio_floor_db: float = -42.0  # radio's own hum/noise re a full-deviation sine (mic path)

    def __post_init__(self) -> None:
        self.if_sos = sg.butter(6, self.if_half_hz, "lowpass", fs=FS, output="sos")
        w, h = sg.sosfreqz(self.if_sos, worN=8192, fs=FS)
        # complex filter noise bandwidth (both sides): 2 * integral over 0..FS/2 of |H|^2
        self.noise_bw = 2 * np.trapezoid(np.abs(h) ** 2, w)

    def tx(self, x: np.ndarray, drive_db: float, precomp: bool) -> np.ndarray:
        """drive_db: RMS at the limiter input (mic) or RMS deviation (data), dB re peak dev."""
        if self.path == "mic":
            s = sg.lfilter(*DE, x) if precomp else x
            s = sg.lfilter(*PRE, sg.sosfilt(TX_HP, s))
            s = s / np.sqrt(np.mean(s**2)) * 10 ** (drive_db / 20)
            s = np.clip(s, -1.0, 1.0)
            return sg.sosfilt(SPLATTER, s)
        s = sg.sosfilt(DC, x)
        return s / np.sqrt(np.mean(s**2)) * 10 ** (drive_db / 20)

    def air(self, m: np.ndarray, cnr_db: float, rng) -> np.ndarray:
        ph = 2 * np.pi * self.dev_hz * np.cumsum(m) / FS
        s = np.exp(1j * ph)
        sigma2 = FS / (self.noise_bw * 10 ** (cnr_db / 10))
        n = np.sqrt(sigma2 / 2) * (rng.standard_normal(len(s)) + 1j * rng.standard_normal(len(s)))
        y = sg.sosfilt(self.if_sos, s + n)
        d = np.angle(y[1:] * np.conj(y[:-1])) * FS / (2 * np.pi * self.dev_hz)
        return np.concatenate([[0.0], d])

    def rx(self, d: np.ndarray, rng) -> np.ndarray:
        if self.path == "mic":
            a = sg.lfilter(*DE, d)
            a = sg.sosfilt(RX_LP, sg.sosfilt(RX_HP, a))
            # the radio's own audio floor, re a full-deviation 1 kHz sine (RMS 1/sqrt2)
            fl = 10 ** (self.audio_floor_db / 20) / np.sqrt(2)
            hum = fl * sg.sosfilt(RX_LP, sg.sosfilt(RX_HP, rng.standard_normal(len(a))))
            hum *= fl / np.sqrt(np.mean(hum**2))
            return a + hum
        return sg.sosfilt(DATA_LP, sg.sosfilt(DC, d))


def measure(x: np.ndarray, r: np.ndarray, band: tuple[float, float]) -> tuple[float, float]:
    """(rate bit/s, effective SNDR dB) over the band, distortion counted as noise."""
    lag = np.argmax(sg.correlate(r[: 2 * FS], x[: 2 * FS], mode="full", method="fft")) - (
        2 * FS - 1
    )
    if lag > 0:
        r, x = r[lag:], x[: len(x) - lag]
    f, C = sg.coherence(x, r, fs=FS, nperseg=4096, noverlap=2048)
    sel = (f >= band[0]) & (f <= band[1])
    C = np.clip(C[sel], 0, 1 - 1e-6)
    sndr = C / (1 - C)
    rate = np.trapezoid(np.log2(1 + sndr), f[sel])
    bw = band[1] - band[0]
    eff = 10 * np.log10(2 ** (rate / bw) - 1)
    return float(rate), float(eff)
