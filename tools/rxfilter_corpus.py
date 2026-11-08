"""A labelled corpus for the receive-filter estimate, and today's estimate judged over it.

The receive-filter review (``docs/research/receive-filter/``), Gate 0 item 3. Each clip is 48 kHz
mono audio as a transceiver's codec delivers it with nobody on the channel: the band's noise
through a receive response whose shape is known — a filter of a given width, the same filter
moved by an IF shift, a low cut set high, a tilt, a notch — over the audio chain's own white
floor, under one condition: steady, an AGC answering a strong signal the audio does not carry,
QSB, a band full of FT8, or audio driven hot enough to clip. The labels say where the response
is 6 dB under its peak and how far down it is at the edges of the signal Aether sends at each
bandwidth (360 and 2640 Hz at 2300 Hz, 1260 and 1740 Hz at 500 Hz).

    python tools/rxfilter_corpus.py make OUT_DIR [--only NAME-PART] [--seed N] [--list]
    python tools/rxfilter_corpus.py judge OUT_DIR [--daemon PATH] [--csv FILE]

``judge`` runs ``aetherd --passband-trace`` over every clip (no sidecar: the whole clip is
listened to) and sets the estimate's last reading beside the truth, with the warning the panel
gives today (the width under the occupied band less 400 Hz).

The building blocks are the busy detector's corpus's (``tools/busy_corpus.py``): its FT8
stations, its AGC model and its WAV writer.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import multiprocessing
import os
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

from busy_corpus import AGC_PRESETS, Source, source_audio, write_wav
from channel_server import Agc

AUDIO_RATE = 48_000
NOISE_3K_DBFS = -30.0
"""The band's noise in 3 kHz of audio, dBFS: a codec set for Setup step 3's *Good*."""
HOT_NOISE_RMS_DBFS = -8.0
"""The whole audio's RMS when the codec is driven hot, dBFS: the mobile on 2026-11-08 (−7 to
−8 dBFS over its recordings, 1.3–1.7 % of samples at full scale)."""
SIGNAL_EDGES_HZ = {2300: (360.0, 2640.0), 500: (1260.0, 1740.0)}
"""The edges of what Aether sends at each bandwidth: its occupied width around 1500 Hz."""
OCCUPIED_HZ = {2300: 2280.0, 500: 480.0}
"""The occupied width the panel's warning compares the estimate with (``occupied_hz``)."""
WARNING_SLACK_HZ = 400.0
"""The panel warns when the estimate is under the occupied width less this (``applyPassband``)."""
FLOORS_DB = {"deep": 50.0, "shallow": 15.0}
"""How far the audio chain's own floor lies under the band's noise, in 3 kHz each: the FTDX10's
about 50 dB, the mobile's TS-480 on 2026-11-08 about 13 dB."""
DEFAULT_DAEMON = Path(__file__).resolve().parents[1] / "core" / "target" / "release" / "aetherd"


@dataclass
class Response:
    """A receive response: a band-pass of ``low``–``high`` Hz (eighth order), then a tilt and
    a notch. ``low`` at or under 30 Hz is a low-pass alone, as a passband moved down by an IF
    shift reaches the bottom of the audio."""

    low: float
    high: float
    tilt_db_per_khz: float = 0.0
    """From 1 kHz up, the response falls this much a kilohertz (a negative tilt rises)."""
    notch: tuple[float, float, float] | None = None
    """(centre Hz, width Hz, depth dB)."""

    def gain_db(self, hz: np.ndarray) -> np.ndarray:
        """The response's power gain at ``hz``, dB."""
        hz = np.maximum(np.asarray(hz, dtype=float), 1e-3)
        order = 8
        high = 1.0 / (1.0 + (hz / self.high) ** (2 * order))
        low = 1.0 if self.low <= 30.0 else 1.0 / (1.0 + (self.low / hz) ** (2 * order))
        db = 10 * np.log10(high * low)
        db -= self.tilt_db_per_khz * np.maximum(hz - 1000.0, 0.0) / 1000.0
        if self.notch is not None:
            centre, width, depth = self.notch
            db -= depth * np.exp(-0.5 * ((hz - centre) / (width / 2.355)) ** 2)
        return db


@dataclass
class Clip:
    """One recording's worth of conditions."""

    name: str
    response: Response
    condition: str  # steady, agc, qsb, ft8, hot
    floor: str  # deep, shallow
    seconds: float = 30.0
    seed: int = 1
    labels: dict = field(default_factory=dict)


RESPONSES: dict[str, Response] = {
    "ssb2700": Response(300.0, 3000.0),
    "ssb2400": Response(300.0, 2700.0),
    "ssb2100": Response(400.0, 2500.0),
    "ssb1800": Response(600.0, 2400.0),
    "narrow500": Response(1250.0, 1750.0),
    "shift-down-300": Response(30.0, 2700.0),
    "shift-down-600": Response(30.0, 2400.0),
    "shift-down-1100": Response(30.0, 1600.0),  # the mobile's TS-480 on 2026-11-08
    "shift-up-300": Response(600.0, 3300.0),
    "shift-up-600": Response(900.0, 3600.0),
    "lowcut600": Response(600.0, 3000.0),
    "tilt": Response(300.0, 3000.0, tilt_db_per_khz=3.0),  # the FTDX10's gentle slope
    "notch1800": Response(300.0, 3000.0, notch=(1800.0, 60.0, 30.0)),
}
CONDITIONS = ("steady", "agc", "qsb", "ft8", "hot")
SECONDS = {"ft8": 150.0, "agc": 120.0}
"""How long a clip runs, by condition (30 s otherwise): the busy detector calls a band of FT8
busy 86 % of the time and an AGC's dips 75 %, and the estimate takes its looks only in the
quiet, so a live station needs a minute or two to read there."""


def truth(response: Response) -> dict:
    """Where the response is 6 dB under its peak (outward from the peak), and how far down it
    is at the edges of the signal at each bandwidth, both relative to its peak (the notch left
    out: a notch inside the passband is not its edge)."""
    plain = Response(response.low, response.high, response.tilt_db_per_khz)
    hz = np.arange(0.0, 4000.0, 1.0)
    db = plain.gain_db(hz)
    peak_at = int(np.argmax(db))
    peak = float(db[peak_at])
    under = db < peak - 6.0
    above = np.nonzero(under[peak_at:])[0]
    below = np.nonzero(under[: peak_at + 1][::-1])[0]
    high = float(peak_at + above[0]) if len(above) else 4000.0
    low = float(peak_at - below[0]) if len(below) else 0.0
    out = {"low_6db": low, "high_6db": high}
    for bandwidth, (a, b) in SIGNAL_EDGES_HZ.items():
        out[f"loss_low_{bandwidth}"] = round(peak - float(plain.gain_db(np.array([a]))[0]), 1)
        out[f"loss_high_{bandwidth}"] = round(peak - float(plain.gain_db(np.array([b]))[0]), 1)
    return out


def corpus(seed: int = 0) -> list[Clip]:
    """Every filter under every condition, over both chain floors."""
    clips = []
    for k, (rname, response) in enumerate(RESPONSES.items()):
        for c, condition in enumerate(CONDITIONS):
            for f, floor in enumerate(FLOORS_DB):
                clips.append(
                    Clip(
                        name=f"{rname}-{condition}-{floor}",
                        response=response,
                        condition=condition,
                        floor=floor,
                        seconds=SECONDS.get(condition, 30.0),
                        seed=1 + seed * 10_000 + k * 100 + c * 10 + f,
                        labels=truth(response),
                    )
                )
    return clips


# ── building the audio ────────────────────────────────────────────────


def shaped(white: np.ndarray, response: Response) -> np.ndarray:
    """White noise through the response, by FFT over the whole clip."""
    spectrum = np.fft.rfft(white)
    hz = np.fft.rfftfreq(len(white), 1.0 / AUDIO_RATE)
    return np.fft.irfft(spectrum * 10 ** (response.gain_db(hz) / 20.0), len(white))


def build(clip: Clip) -> tuple[np.ndarray, dict]:
    """The clip's audio and its labels."""
    n = int(clip.seconds * AUDIO_RATE)
    rng = np.random.default_rng(clip.seed)
    band = shaped(rng.standard_normal(n), clip.response)
    # the band's noise at NOISE_3K_DBFS in 3 kHz of the response's flat top
    flat = Response(clip.response.low, clip.response.high)
    hz = np.fft.rfftfreq(n, 1.0 / AUDIO_RATE)
    top = float(np.max(flat.gain_db(hz)))
    white_3k = 3000.0 / (AUDIO_RATE / 2.0)  # the share of a white noise's power in 3 kHz
    band *= 10 ** ((NOISE_3K_DBFS - top) / 20.0) / math.sqrt(white_3k)
    felt = np.zeros(n)
    t = np.arange(n) / AUDIO_RATE
    if clip.condition == "qsb":
        band *= 10 ** (6.0 * np.sin(2 * np.pi * t / 8.0) / 20.0)
    if clip.condition == "ft8":
        srng = np.random.default_rng(clip.seed + 3)
        for k in range(10):
            source = Source("ft8", float(srng.uniform(400.0, 2600.0)), float(srng.uniform(3, 15)))
            clean = source_audio(source, n, clip.seed * 100 + k)
            band += shaped(clean, clip.response) * 10 ** ((NOISE_3K_DBFS + source.snr_3k_db) / 20.0)
    if clip.condition == "agc":
        # a strong station the roofing filter passes and the DSP filter does not: the AGC
        # answers to it and takes the band's noise down with it, the chain's floor not
        noise_rms_3k = 10 ** (NOISE_3K_DBFS / 20.0)
        windows = [(float(s), float(s) + 1.5) for s in np.arange(2.0, clip.seconds, 5.0)]
        source = Source("carrier", 3600.0, 30.0, windows=windows)
        felt = source_audio(source, n, clip.seed) * noise_rms_3k * 10 ** (30.0 / 20.0)
        hang, decay = AGC_PRESETS["mid"]
        agc = Agc(noise_rms_3k * 10 ** (12.0 / 20.0), hang, decay)
        steps = n // Agc.STEP * Agc.STEP
        gains = np.ones(n)
        gains[:steps] = np.repeat(10 ** (agc.gains_db((band + felt)[:steps]) / 20.0), Agc.STEP)
        band *= gains
    floor_3k_dbfs = NOISE_3K_DBFS - FLOORS_DB[clip.floor]
    chain = rng.standard_normal(n) * 10 ** (floor_3k_dbfs / 20.0) / math.sqrt(white_3k)
    audio = band + chain
    if clip.condition == "hot":
        audio *= 10 ** (HOT_NOISE_RMS_DBFS / 20.0) / math.sqrt(np.mean(audio**2))
    labels = {
        "name": clip.name,
        "response": clip.response.__dict__,
        "condition": clip.condition,
        "floor": clip.floor,
        "floor_db_under_noise": FLOORS_DB[clip.floor],
        "seconds": clip.seconds,
        "clipped_share": float(np.mean(np.abs(audio) >= 1.0)),
        **clip.labels,
    }
    return np.clip(audio, -1.0, 1.0), labels


def write_clip(job: tuple[Clip, Path]) -> str:
    clip, out = job
    audio, labels = build(clip)
    write_wav(out / f"{clip.name}.wav", audio)
    (out / f"{clip.name}.json").write_text(json.dumps(labels, indent=1), encoding="utf-8")
    return clip.name


# ── judging today's estimate ──────────────────────────────────────────


def last_reading(daemon: Path, wav: Path, bandwidth: int) -> dict | None:
    """The estimate's last reading over a clip, from ``aetherd --passband-trace``."""
    out = subprocess.run(
        [str(daemon), "--passband-trace", str(wav), "--bandwidth", str(bandwidth)],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    rows = [r for r in csv.DictReader(out.splitlines()) if r["width_hz"]]
    return rows[-1] if rows else None


def judge(out: Path, daemon: Path, table: Path | None) -> None:
    rows = []
    for labels_path in sorted(out.glob("*.json")):
        labels = json.loads(labels_path.read_text(encoding="utf-8"))
        reading = last_reading(daemon, labels_path.with_suffix(".wav"), 2300)
        row = {
            "clip": labels["name"],
            "condition": labels["condition"],
            "floor": labels["floor"],
            "true_low": labels["low_6db"],
            "true_high": labels["high_6db"],
            "loss_low_2300": labels["loss_low_2300"],
            "loss_high_2300": labels["loss_high_2300"],
            "loss_low_500": labels["loss_low_500"],
            "loss_high_500": labels["loss_high_500"],
            "clipped_share": round(labels["clipped_share"], 4),
        }
        if reading is None:
            row.update(est_low="", est_high="", est_width="", looks=0, warns_2300="")
        else:
            width = float(reading["width_hz"])
            row.update(
                est_low=float(reading["low_hz"]),
                est_high=float(reading["high_hz"]),
                est_width=width,
                looks=int(reading["looks"]),
                warns_2300=int(width < OCCUPIED_HZ[2300] - WARNING_SLACK_HZ),
            )
        rows.append(row)
        print(
            f"{row['clip']:32s} true {row['true_low']:5.0f}-{row['true_high']:5.0f}"
            f"  est {row['est_low']!s:>6}-{row['est_high']!s:<6}"
            f" lost at 2300 Hz's edges {row['loss_low_2300']:5.1f}/{row['loss_high_2300']:5.1f} dB"
            f"  warns {row['warns_2300']}",
            flush=True,
        )
    if table is not None:
        with table.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    make = sub.add_parser("make", help="build the clips")
    make.add_argument("out", type=Path)
    make.add_argument("--only", help="build only clips whose name contains this")
    make.add_argument("--list", action="store_true", help="list the clips and stop")
    make.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    make.add_argument("--seed", type=int, default=0, help="draw the clips again")
    jud = sub.add_parser("judge", help="today's estimate over the clips")
    jud.add_argument("out", type=Path)
    jud.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    jud.add_argument("--csv", type=Path)
    args = ap.parse_args()
    if args.command == "judge":
        judge(args.out, args.daemon, args.csv)
        return
    clips = [c for c in corpus(args.seed) if not args.only or args.only in c.name]
    if args.list:
        for clip in clips:
            print(clip.name, json.dumps(clip.labels))
        return
    args.out.mkdir(parents=True, exist_ok=True)
    with multiprocessing.Pool(args.jobs) as pool:
        for name in pool.imap_unordered(write_clip, [(c, args.out) for c in clips]):
            print(name, flush=True)


if __name__ == "__main__":
    main()
