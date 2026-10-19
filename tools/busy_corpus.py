"""A labelled corpus for the busy detector: the radio's audio as an AGC delivers it (ADR-0078).

Each clip is 48 kHz mono audio as a transceiver's USB codec delivers it: the band's noise
through the radio's filter, other stations at chosen levels, frequencies and times, and the
radio's AGC acting on the whole passband, as a real one does — a strong signal anywhere in the
passband turns everything down together. Beside each clip go the same noise after the same
gain (so the true noise floor is known at every block) and a JSON file of labels: every signal,
where it sat, whether it was inside Aether's channel, its SNR, and when it was on.

The signals are the scenario harness's own (:mod:`aether_model.qrm`), keyed on fixed windows
where an experiment needs to say exactly when something comes and goes. The AGC is the
harness's model (``tools/channel_server.py``'s ``Agc``: a step down within a millisecond, a
hold, a ramp back), with presets for the settings of the station whose recordings the failure
came from; they are provisional until the recordings fit them (``02-measurements.md``).

    python tools/busy_corpus.py OUT_DIR [--only NAME-PART] [--list] [--seed N]

``--seed N`` draws every clip again with other random numbers — the same conditions, other noise
and other signals' timing: the held-out corpus a candidate tuned on the first is confirmed on.

``tools/busy_eval.py`` runs the detector's candidates over the corpus and scores them.
"""

from __future__ import annotations

import argparse
import json
import math
import multiprocessing
import os
import sys
import wave
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from scipy import signal

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from aether_model.qrm import (
    AtmosphericCrashes,
    Carrier,
    Ft8Station,
    OfdmArqStation,
    PactorStation,
    RttyStation,
)
from channel_server import Agc

AUDIO_RATE = 48_000
BASEBAND_RATE = 8_000
CENTRE_HZ = 1500.0
BLOCK_S = 0.025
NOISE_3K_DBFS = -24.0
"""The band's noise in 3 kHz of audio, dBFS: about −29 dBFS in a 500 Hz channel at the detector,
where the author's station reads −27."""
RADIO_FILTER_HZ = (250.0, 2950.0)
"""A 2.7 kHz SSB/DATA filter: what the radio passes and what its AGC acts on."""
CODEC_FLOOR_DBFS = -92.0
"""The audio chain's own noise after the AGC, which a deep cut cannot push the noise below."""
CHANNEL_HZ = {500: 480.0, 2300: 2280.0}
"""Aether's occupied bandwidth at each bandwidth, centred on 1500 Hz."""
HEARD_BEYOND_HZ = 280.0
"""How far past the occupied band the detector's band filter still lets energy through: a
subcarrier and the filter's transition (``aether_phy::passband``). A signal there is in the
channel as the detector hears it."""
AGC_PRESETS = {
    # hang (ms), decay (dB/s); provisional, to be fitted to the author's recordings
    "fast": (50.0, 120.0),
    "mid": (100.0, 50.0),  # beta.17's measurement of an FTDX10 on AUTO in DATA (MID)
    "slow": (200.0, 15.0),
    "ssb-slow": (300.0, 7.0),  # AUTO in USB: SLOW, 3 s
}
AGC_THRESHOLD_DB = 12.0
"""Where the AGC begins to act, above the band noise's RMS in 3 kHz: band noise alone leaves the
gain alone, as a rig's S-meter sits at S0–S2 on noise."""


@dataclass
class Source:
    """One signal in a clip."""

    kind: str  # carrier, rtty, pactor, ft8, ofdm500, ofdm2400, cw, crashes
    audio_hz: float
    snr_3k_db: float
    """Its power over the band noise's power in 3 kHz."""
    windows: list[tuple[float, float]] | None = None
    """On exactly through these, or its own rhythm when None."""
    seed: int = 0
    hidden: bool = False
    """Felt by the AGC but not in the audio: a signal inside the radio's roofing filter and
    outside its DSP filter, which a gain loop ahead of the narrow filter answers to (the
    survey, §3.8; beta.17's recording: dips "triggered outside the audio passband")."""


@dataclass
class Clip:
    """One recording's worth of conditions."""

    name: str
    group: str
    seconds: float
    bandwidth: int = 500
    agc: str = "off"
    sources: list[Source] = field(default_factory=list)
    radio_filter: tuple[float, float] = RADIO_FILTER_HZ
    noise_steps: list[tuple[float, float]] = field(default_factory=list)
    """The band noise moving for real: (from second, dB), held until the next."""
    seed: int = 1


# ── building the audio ────────────────────────────────────────────────


def to_audio(baseband: np.ndarray) -> np.ndarray:
    """Complex baseband at 8 kHz, centred on the modem's audio centre, as 48 kHz audio of the
    same power."""
    up = signal.resample_poly(baseband, AUDIO_RATE // BASEBAND_RATE, 1)
    t = np.arange(len(up)) / AUDIO_RATE
    return np.real(up * np.exp(2j * np.pi * CENTRE_HZ * t)) * math.sqrt(2.0)


def band_noise(n: int, rng: np.random.Generator, band: tuple[float, float]) -> np.ndarray:
    """White noise through the radio's filter, scaled to unit power in 3 kHz."""
    sos = signal.butter(8, band, btype="bandpass", fs=AUDIO_RATE, output="sos")
    x = signal.sosfilt(sos, rng.standard_normal(n + 4800))[4800:]
    width = band[1] - band[0]
    return x / math.sqrt(np.mean(x**2)) * math.sqrt(3000.0 / width)


def morse_envelope(n: int, windows: list[tuple[float, float]], seed: int) -> np.ndarray:
    """CW at 20 wpm sending random characters through each window: keyed about half the time,
    5 ms rise and fall."""
    rng = np.random.default_rng(seed)
    dot = 1.2 / 20.0
    env = np.zeros(n)
    for start, end in windows:
        t = start
        while t < end:
            for _ in range(int(rng.integers(1, 5))):
                length = dot if rng.random() < 0.5 else 3 * dot
                a, b = int(t * AUDIO_RATE), int(min(t + length, end) * AUDIO_RATE)
                env[a:b] = 1.0
                t += length + dot
            t += 2 * dot  # between characters
            if rng.random() < 0.2:
                t += 4 * dot  # between words
    ramp = np.ones(int(0.005 * AUDIO_RATE)) / int(0.005 * AUDIO_RATE)
    return np.convolve(env, ramp, mode="same")


def source_audio(src: Source, n: int, seed: int) -> np.ndarray:
    """A source's clean audio at unit power while on (its own power: 0 dB)."""
    nb = n // (AUDIO_RATE // BASEBAND_RATE) + 64
    offset = src.audio_hz - CENTRE_HZ
    kw = {"seed": seed, "windows": src.windows}
    if src.kind == "cw":
        t = np.arange(n) / AUDIO_RATE
        tone = math.sqrt(2.0) * np.cos(2 * np.pi * src.audio_hz * t)
        return tone * morse_envelope(n, src.windows or [(0.0, n / AUDIO_RATE)], seed)
    if src.kind == "carrier":
        x = Carrier(BASEBAND_RATE, offset, 0.0, **kw).next(nb)  # type: ignore[arg-type]
    elif src.kind == "rtty":
        x = RttyStation(BASEBAND_RATE, offset, 0.0, **kw).next(nb)  # type: ignore[arg-type]
    elif src.kind == "pactor":
        x = PactorStation(BASEBAND_RATE, offset, 0.0, partner_db=0.0, **kw).next(nb)  # type: ignore[arg-type]
    elif src.kind == "ft8":
        x = Ft8Station(BASEBAND_RATE, offset, 0.0, duty=1.0, **kw).next(nb)  # type: ignore[arg-type]
    elif src.kind == "ofdm500":
        # an Aether- or VARA-500-class signal: 10 carriers 50 Hz apart, flat, in ARQ bursts
        x = OfdmArqStation(
            BASEBAND_RATE,
            offset,
            0.0,
            partner_db=0.0,
            carriers=10,
            **kw,  # type: ignore[arg-type]
        ).next(nb)
    elif src.kind == "ofdm2400":
        x = OfdmArqStation(BASEBAND_RATE, offset, 0.0, partner_db=0.0, **kw).next(nb)  # type: ignore[arg-type]
    else:
        raise SystemExit(f"no source kind {src.kind!r}")
    return to_audio(x)[:n]


def on_segments(clean: np.ndarray, merge_s: float = 1.0) -> list[tuple[float, float]]:
    """When a source was on, from its clean audio: 25 ms blocks above a hundredth of its
    loudest, gaps under ``merge_s`` closed."""
    block = int(BLOCK_S * AUDIO_RATE)
    power = np.mean(clean[: len(clean) // block * block].reshape(-1, block) ** 2, axis=1)
    if not power.any():
        return []
    on = power > 0.01 * np.percentile(power[power > 0], 90)
    out: list[list[float]] = []
    for i, flag in enumerate(on):
        if not flag:
            continue
        t0, t1 = i * BLOCK_S, (i + 1) * BLOCK_S
        if out and t0 - out[-1][1] < merge_s:
            out[-1][1] = t1
        else:
            out.append([t0, t1])
    return [(a, b) for a, b in out if b - a >= 0.2]


def in_channel(src: Source, bandwidth: int) -> bool:
    """Whether a source puts energy into Aether's channel as the detector hears it."""
    if src.hidden:
        return False
    half = CHANNEL_HZ[bandwidth] / 2.0 + HEARD_BEYOND_HZ
    widths = {
        "carrier": 0,
        "cw": 0,
        "rtty": 170,
        "pactor": 500,
        "ft8": 50,
        "ofdm500": 500,
        "ofdm2400": 2400,
    }
    half_source = widths[src.kind] / 2.0
    return abs(src.audio_hz - CENTRE_HZ) < half + half_source


def build(clip: Clip) -> tuple[np.ndarray, np.ndarray, dict]:
    """The clip's audio, its noise after the same gain, and its labels."""
    n = int(clip.seconds * AUDIO_RATE)
    rng = np.random.default_rng(clip.seed)
    noise_rms_3k = 10 ** (NOISE_3K_DBFS / 20.0)
    noise = band_noise(n, rng, clip.radio_filter) * noise_rms_3k
    if clip.noise_steps:
        level = np.zeros(n)
        for at, db in clip.noise_steps:
            level[int(at * AUDIO_RATE) :] = db
        noise = noise * 10 ** (level / 20.0)
    mix = noise.copy()
    felt = np.zeros(n)  # what the AGC answers to and the audio does not carry
    labels = []
    sos = signal.butter(8, clip.radio_filter, btype="bandpass", fs=AUDIO_RATE, output="sos")
    for k, src in enumerate(clip.sources):
        seed = clip.seed * 100 + k + src.seed
        if src.kind == "crashes":
            nb = n // 6 + 64
            crash = AtmosphericCrashes(
                BASEBAND_RATE, 1.0, rate_per_s=1.0, peak_db=src.snr_3k_db, seed=seed
            ).next(nb)
            clean = to_audio(crash)[:n] * noise_rms_3k
            mix += signal.sosfilt(sos, clean)
            continue
        clean = source_audio(src, n, seed)
        amplitude = noise_rms_3k * 10 ** (src.snr_3k_db / 20.0)
        if src.hidden:
            felt += clean * amplitude
        else:
            clean = signal.sosfilt(sos, clean) * amplitude
            mix += clean
        labels.append(
            {
                "kind": src.kind,
                "audio_hz": src.audio_hz,
                "snr_3k_db": src.snr_3k_db,
                "in_channel": in_channel(src, clip.bandwidth),
                "hidden": src.hidden,
                "windows": src.windows,
                "segments": on_segments(clean * amplitude if src.hidden else clean),
            }
        )
    gains = np.ones(n)
    if clip.agc != "off":
        hang, decay = AGC_PRESETS[clip.agc]
        agc = Agc(noise_rms_3k * 10 ** (AGC_THRESHOLD_DB / 20.0), hang, decay)
        steps = n // Agc.STEP * Agc.STEP
        gains[:steps] = np.repeat(10 ** (agc.gains_db((mix + felt)[:steps]) / 20.0), Agc.STEP)
    codec = np.random.default_rng(clip.seed + 7).standard_normal(n) * 10 ** (CODEC_FLOOR_DBFS / 20)
    heard = mix * gains + codec
    heard_noise = noise * gains + codec
    # with the AGC off, the operator rides the RF gain so the codec does not clip: everything
    # down together, every level relative to every other unchanged
    peak = float(np.max(np.abs(heard)))
    if clip.agc == "off" and peak > 0.9:
        heard *= 0.9 / peak
        heard_noise *= 0.9 / peak
    meta = {
        "name": clip.name,
        "group": clip.group,
        "seconds": clip.seconds,
        "bandwidth": clip.bandwidth,
        "agc": clip.agc,
        "agc_preset": AGC_PRESETS.get(clip.agc),
        "radio_filter": clip.radio_filter,
        "noise_3k_dbfs": NOISE_3K_DBFS,
        "noise_steps": clip.noise_steps,
        "sources": labels,
        "gain_db_min": float(20 * np.log10(np.min(gains))),
        "peak": float(np.max(np.abs(heard))),
    }
    return heard, heard_noise, meta


def write_wav(path: Path, audio: np.ndarray) -> None:
    pcm = np.clip(np.round(audio * 32767.0), -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(AUDIO_RATE)
        w.writeframes(pcm.tobytes())


# ── the corpus ────────────────────────────────────────────────────────


def corpus() -> list[Clip]:
    """Every clip, by group. The groups are the questions of ``03-candidates.md``."""
    clips: list[Clip] = []
    # A: a strong signal outside the channel holds the AGC down, three times: the failure
    events = [(40.0, 60.0), (100.0, 120.0), (160.0, 180.0)]
    for agc in ("fast", "mid", "slow", "ssb-slow"):
        for kind, hz in (("carrier", 700.0), ("carrier", 2400.0), ("rtty", 2400.0), ("ft8", 800.0)):
            for snr in (25.0, 40.0):
                clips.append(
                    Clip(
                        f"A-{kind}{int(hz)}-{int(snr)}-{agc}",
                        "A",
                        220.0,
                        agc=agc,
                        sources=[Source(kind, hz, snr, events)],
                        seed=len(clips) + 1,
                    )
                )
    # A at 2300 Hz: the only room outside the channel is the filter's edge
    for agc in ("mid", "slow"):
        for snr in (25.0, 40.0):
            clips.append(
                Clip(
                    f"A2300-carrier2800-{int(snr)}-{agc}",
                    "A2300",
                    220.0,
                    bandwidth=2300,
                    agc=agc,
                    sources=[Source("carrier", 2800.0, snr, events)],
                    seed=len(clips) + 1,
                )
            )
    # B: a signal in the channel, alone: is it called busy, and how fast
    for agc in ("off", "mid"):
        for kind in ("carrier", "cw", "rtty", "pactor", "ft8", "ofdm500", "ofdm2400"):
            for snr in (-6.0, 0.0, 6.0, 12.0, 20.0):
                clips.append(
                    Clip(
                        f"B-{kind}-{int(snr):+d}-{agc}",
                        "B",
                        70.0,
                        agc=agc,
                        sources=[Source(kind, 1500.0, snr, [(20.0, 50.0)])],
                        seed=len(clips) + 1,
                    )
                )
    # B at 2300 Hz: wide signals in a wide channel
    for kind in ("ofdm2400", "rtty", "pactor"):
        for snr in (0.0, 6.0, 12.0):
            clips.append(
                Clip(
                    f"B2300-{kind}-{int(snr):+d}-mid",
                    "B2300",
                    70.0,
                    bandwidth=2300,
                    agc="mid",
                    sources=[Source(kind, 1500.0, snr, [(20.0, 50.0)])],
                    seed=len(clips) + 1,
                )
            )
    # C: a signal in the channel while a strong one outside holds the AGC down
    for agc in ("mid", "slow"):
        for kind in ("rtty", "ofdm500", "cw"):
            for snr in (0.0, 6.0, 12.0):
                clips.append(
                    Clip(
                        f"C-{kind}-{int(snr):+d}-{agc}",
                        "C",
                        110.0,
                        agc=agc,
                        sources=[
                            Source("carrier", 700.0, 40.0, [(20.0, 90.0)]),
                            Source(kind, 1500.0, snr, [(45.0, 70.0)]),
                        ],
                        seed=len(clips) + 1,
                    )
                )
    # D: no AGC trouble at all: the floor and false alarms must not change
    for agc in ("off", "fast", "mid"):
        clips.append(Clip(f"D-noise-{agc}", "D", 150.0, agc=agc, seed=len(clips) + 1))
        clips.append(
            Clip(
                f"D-crashes-{agc}",
                "D",
                150.0,
                agc=agc,
                sources=[Source("crashes", 0.0, 30.0)],
                seed=len(clips) + 1,
            )
        )
        # a band of FT8 beside the channel, on its own rhythm
        clips.append(
            Clip(
                f"D-ft8band-{agc}",
                "D",
                150.0,
                agc=agc,
                sources=[
                    Source("ft8", hz, snr, None, seed=k)
                    for k, (hz, snr) in enumerate(
                        [
                            (500, 15),
                            (650, 22),
                            (800, 10),
                            (950, 30),
                            (2100, 18),
                            (2350, 25),
                            (2550, 12),
                        ]
                    )
                ],
                seed=len(clips) + 1,
            )
        )
    # E: a long flat signal over the channel: busy for as long as the detector promises
    for agc in ("off", "mid"):
        for snr in (3.0, 9.0, 15.0):
            clips.append(
                Clip(
                    f"E-ofdm2400-{int(snr)}-{agc}",
                    "E",
                    150.0,
                    agc=agc,
                    sources=[Source("ofdm2400", 1500.0, snr, [(20.0, 140.0)])],
                    seed=len(clips) + 1,
                )
            )
            clips.append(
                Clip(
                    f"E-ofdm500-{int(snr)}-{agc}",
                    "E",
                    150.0,
                    agc=agc,
                    sources=[Source("ofdm500", 1500.0, snr, [(20.0, 140.0)])],
                    seed=len(clips) + 1,
                )
            )
    # F: a narrow radio filter around the channel: only the channel drives the AGC
    for agc in ("mid", "slow", "ssb-slow"):
        clips.append(
            Clip(
                f"F-narrow-rtty-30-{agc}",
                "F",
                150.0,
                agc=agc,
                radio_filter=(1150.0, 1850.0),
                sources=[Source("rtty", 1500.0, 30.0, [(30.0, 60.0), (90.0, 110.0)])],
                seed=len(clips) + 1,
            )
        )
    # F': the strong signal is in the channel, wide filter: the AGC's hold after it
    for agc in ("mid", "slow", "ssb-slow"):
        clips.append(
            Clip(
                f"F-wide-rtty-30-{agc}",
                "F",
                150.0,
                agc=agc,
                sources=[Source("rtty", 1500.0, 30.0, [(30.0, 60.0), (90.0, 110.0)])],
                seed=len(clips) + 1,
            )
        )
    # H: a signal the AGC feels and the audio never carries, at both bandwidths
    for bandwidth in (500, 2300):
        for agc in ("mid", "slow", "ssb-slow"):
            for snr in (30.0, 45.0):
                clips.append(
                    Clip(
                        f"H{bandwidth}-hidden-{int(snr)}-{agc}",
                        "H",
                        220.0,
                        bandwidth=bandwidth,
                        agc=agc,
                        sources=[Source("carrier", 3300.0, snr, events, hidden=True)],
                        seed=len(clips) + 1,
                    )
                )
    # L: a long cut, longer than the floor's window and hold together: a broadcast carrier
    # beside the channel for two minutes, seen and unseen
    for bandwidth, kind, hz, hidden in (
        (500, "carrier", 700.0, False),
        (500, "carrier", 3300.0, True),
        (2300, "carrier", 3300.0, True),
    ):
        for agc in ("mid", "slow"):
            clips.append(
                Clip(
                    f"L{bandwidth}-{'hidden' if hidden else kind + str(int(hz))}-{agc}",
                    "L",
                    220.0,
                    bandwidth=bandwidth,
                    agc=agc,
                    sources=[Source(kind, hz, 40.0, [(30.0, 150.0)], hidden=hidden)],
                    seed=len(clips) + 1,
                )
            )
    # C2300: a wide signal in a 2300 Hz channel while a hidden one holds the AGC down — the
    # case the reference cannot leave the channel out of
    for snr in (3.0, 9.0, 15.0):
        clips.append(
            Clip(
                f"C2300-ofdm2400-{int(snr)}-mid",
                "C2300",
                110.0,
                bandwidth=2300,
                agc="mid",
                sources=[
                    Source("carrier", 3300.0, 40.0, [(20.0, 90.0)], hidden=True),
                    Source("ofdm2400", 1500.0, snr, [(45.0, 70.0)]),
                ],
                seed=len(clips) + 1,
            )
        )
    # G: the band's noise really steps, no AGC: no worse than before
    for step in (10.0, -10.0):
        clips.append(
            Clip(
                f"G-noise-step{int(step):+d}",
                "G",
                150.0,
                agc="off",
                noise_steps=[(60.0, step)],
                seed=len(clips) + 1,
            )
        )
    return clips


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("out", type=Path)
    ap.add_argument("--only", help="build only clips whose name contains this")
    ap.add_argument("--list", action="store_true", help="list the clips and stop")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1, help="clips built at once")
    ap.add_argument("--seed", type=int, default=0, help="draw the clips again with other numbers")
    args = ap.parse_args()
    clips = [c for c in corpus() if not args.only or args.only in c.name]
    for c in clips:
        c.seed += 1000 * args.seed
    if args.list:
        for c in clips:
            print(f"{c.name:34} {c.group:6} {c.seconds:5.0f} s  agc {c.agc}")
        print(f"{len(clips)} clips, {sum(c.seconds for c in clips) / 60:.0f} min")
        return
    args.out.mkdir(parents=True, exist_ok=True)
    jobs = [(c, args.out) for c in clips]
    with multiprocessing.Pool(args.jobs) as pool:
        for line in pool.imap(write_clip, jobs):
            print(line, flush=True)


def write_clip(job: tuple[Clip, Path]) -> str:
    """Build one clip and write its three files."""
    clip, out = job
    heard, noise, meta = build(clip)
    write_wav(out / f"{clip.name}.wav", heard)
    write_wav(out / f"{clip.name}.noise.wav", noise)
    (out / f"{clip.name}.json").write_text(json.dumps(meta, indent=1))
    return f"{clip.name:34} gain min {meta['gain_db_min']:6.1f} dB  peak {meta['peak']:.2f}"


if __name__ == "__main__":
    main()
