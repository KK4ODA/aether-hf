"""The receivers' noise shape on paths no filter cuts: the same frames decoded with the noise read
flat and read across the band (the receive-filter review; the receiver change it put first).

    python -I tools/bench_noise_shape.py OUT.csv --family ofdm|tone [--shapes flat,shaped]
        [--frames 30] [--jobs 4] [--channels awgn,good,moderate,poor,flutter,nvis]

A receiver that reads the noise per carrier or per tone must not lose where the noise is flat
(ADR-0104). ``flat`` turns the reading off (``FrameReceiver.noise_shape``, ``tone.NOISE_SHAPE``),
``shaped`` leaves it on. The points are taken from the committed curves — ``phy_fer.csv`` and
``phy_fer_500.csv`` for the OFDM modes, ``tone_floor.csv`` for the tone kinds: for every air or
kind, channel and mode, the SNR nearest 30 % frame errors and the one nearest 10 %, where a
change in the decoder shows first. Flutter and ITU NVIS, which those curves do not hold, take
Poor's points — NVIS above all: reading a fast or four-tone kind's data tone by tone lost there
and nowhere else (ADR-0104 §3). Each point's frames go through the family's own bench (``bench_phy.run_point``: random
carrier offset, sample-rate offset, timing and fading seed per frame; ``bench_tone.run``: random
start and carrier offset, through the detector and with genie timing) once for each shape, so
the shapes see the same frames, the same channel and the same noise. The CSV is one row a point
and shape; the summary, the decodes of each shape point by point and in all.
"""

from __future__ import annotations

import argparse
import csv
import sys
from collections import defaultdict
from multiprocessing import Pool
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "model"))
sys.path.insert(0, str(ROOT / "tools"))

MODES = {2300: (2, 4, 6, 8, 10, 13), 500: (3, 6, 9, 12)}
"""OFDM modes the committed curves hold: wide rungs 8, 10, 12, 14, 16, 19; narrow rungs 5, 8, 11,
14, as the cut bench."""
BASELINES = {2300: "bench/baselines/phy_fer.csv", 500: "bench/baselines/phy_fer_500.csv"}
TONE_BASELINE = "bench/baselines/tone_floor.csv"
TARGETS = (0.3, 0.1)
NO_CURVE = {"flutter": "poor", "nvis": "poor"}
"""Channels the committed curves do not hold, and whose points they take."""


def nearest(curve: list[tuple[float, float]]) -> list[float]:
    """The SNRs of ``(snr, fer)`` nearest each target frame error rate."""
    return sorted({min(curve, key=lambda p, t=t: abs(p[1] - t))[0] for t in TARGETS})


def ofdm_points(channels: list[str]) -> list[tuple[str, str, str, float]]:
    """``(air, channel, mode, snr)`` for every air, channel and mode."""
    out = []
    for hz, path in BASELINES.items():
        curve: dict[tuple[str, int], list[tuple[float, float]]] = defaultdict(list)
        for r in csv.DictReader((ROOT / path).open(encoding="utf-8")):
            curve[(r["channel"], int(r["mode"]))].append((float(r["snr_3k_db"]), float(r["fer"])))
        for channel in channels:
            for mode in MODES[hz]:
                pts = curve.get((NO_CURVE.get(channel, channel), mode))
                out.extend((str(hz), channel, str(mode), s) for s in nearest(pts or []))
    return out


def tone_points(channels: list[str]) -> list[tuple[str, str, str, float]]:
    """``(air, channel, kind, snr)`` for every tone kind and channel, by the detector's decodes."""
    from bench_tone import AIRS

    curve: dict[tuple[str, str], list[tuple[float, float]]] = defaultdict(list)
    for r in csv.DictReader((ROOT / TONE_BASELINE).open(encoding="utf-8")):
        fer = 1.0 - int(r["decoded"]) / int(r["frames"])
        curve[(r["channel"], r["frame"])].append((float(r["snr_3k_db"]), fer))
    out = []
    for kind in sorted({k for _, k in curve}):
        hz = str(AIRS[kind].params.bandwidth.value)
        for channel in channels:
            pts = curve.get((NO_CURVE.get(channel, channel), kind))
            out.extend((hz, channel, kind, s) for s in nearest(pts or []))
    return out


def run(job: tuple[str, str, str, str, float, int, int, str]) -> dict[str, object]:
    family, hz, channel, what, snr, frames, seed, shape = job
    if family == "ofdm":
        from aether_model.phy.pipeline import Modem
        from aether_model.phy.rx import FrameReceiver
        from aether_model.waveform import NARROW_500, WIDE_2300
        from bench_phy import run_point

        FrameReceiver.noise_shape = shape == "shaped"
        modem = Modem(WIDE_2300 if hz == "2300" else NARROW_500)
        row = run_point(modem, channel, int(what), snr, frames, seed)
        decoded, genie = int(row["decoded"]), ""
    else:
        import bench_tone
        from aether_model.phy import tone

        tone.NOISE_SHAPE = shape == "shaped"
        row = bench_tone.run((what, channel, int(snr), frames, seed))
        decoded, genie = int(str(row["decoded"])), str(row["genie_decoded"])
    return {
        "air": hz,
        "channel": channel,
        "what": what,
        "snr_3k_db": snr,
        "shape": shape,
        "frames": frames,
        "decoded": decoded,
        "genie_decoded": genie,
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--family", choices=("ofdm", "tone"), required=True)
    ap.add_argument("--shapes", default="flat,shaped")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--channels", default="awgn,good,moderate,poor,flutter,nvis")
    ap.add_argument("--seed", type=int, default=9000)
    args = ap.parse_args()
    shapes = args.shapes.split(",")
    channels = args.channels.split(",")
    pts = ofdm_points(channels) if args.family == "ofdm" else tone_points(channels)
    jobs = [
        (args.family, hz, channel, what, snr, args.frames, args.seed + 7919 * i, shape)
        for i, (hz, channel, what, snr) in enumerate(pts)
        for shape in shapes
    ]
    rows = []
    with open(args.out, "w", newline="", encoding="utf-8") as f, Pool(args.jobs) as pool:
        w = None
        for row in pool.imap_unordered(run, jobs):
            if w is None:
                w = csv.DictWriter(f, fieldnames=list(row))
                w.writeheader()
            w.writerow(row)
            f.flush()
            rows.append(row)
            print(
                f"{row['air']:>5} {row['channel']:9} {row['what']:>13} {row['snr_3k_db']:+6.1f} dB"
                f" {row['shape']:7} {row['decoded']:3}/{row['frames']} {row['genie_decoded']}",
                flush=True,
            )
    summary(rows, shapes)


def summary(rows: list[dict[str, object]], shapes: list[str]) -> None:
    by: dict[tuple[str, ...], dict[str, int]] = defaultdict(dict)
    for r in rows:
        key = (
            str(r["air"]),
            str(r["channel"]),
            str(r["what"]),
            f"{float(str(r['snr_3k_db'])):+.1f}",
        )
        by[key][str(r["shape"])] = int(str(r["decoded"]))
    print("\ndecoded, by shape", shapes, "(air channel what snr)")
    totals: dict[tuple[str, str], list[int]] = defaultdict(lambda: [0] * len(shapes))
    for key in sorted(by):
        counts = [by[key].get(s, 0) for s in shapes]
        for i, c in enumerate(counts):
            totals[(key[0], key[1])][i] += c
        mark = "" if len(set(counts)) == 1 else "  *"
        print(f"  {key[0]:>4} {key[1]:9} {key[2]:>13} {key[3]:>6}: {counts}{mark}")
    print("\nby air and channel:")
    for key, counts in sorted(totals.items()):
        print(f"  {key[0]:>4} {key[1]:9} {dict(zip(shapes, counts, strict=True))}")


if __name__ == "__main__":
    main()
