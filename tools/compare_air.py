"""Measured against predicted: what a recorded session did, next to what the simulator says.

    python tools/compare_air.py field/sessions/<name>.json [--channel awgn|good|moderate|poor]
                                [--baselines bench/baselines]

Field validation (roadmap Phase 6) ends with "simulator-vs-air throughput within 20 %", and
this is the comparison. From the sidecar it takes what the receiving station measured —
the frames it heard, their modes and SNRs, which decoded, and the bytes delivered over the
session — and puts beside each number what the committed benchmark curves predict at the
same SNR: the AWGN frame error rate per mode (`phy_fer_awgn14.csv`) and the link goodput per
channel class (`link_throughput.csv`), interpolated. The channel class is the operator's call
(`--channel`); without it every class is shown, and the one the air was closest to is the
one to write down.

A session that disagrees with the simulator by more than 20 % is not a failure of either —
it is the number the roadmap says to recalibrate on, once there are enough of them.
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics
import sys
from collections import defaultdict
from itertools import pairwise
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLASSES = ("awgn", "good", "moderate", "poor")


def interpolate(points: list[tuple[float, float]], x: float) -> float | None:
    """Linear interpolation on sorted (x, y); clamped to the ends."""
    if not points:
        return None
    points = sorted(points)
    if x <= points[0][0]:
        return points[0][1]
    if x >= points[-1][0]:
        return points[-1][1]
    for (x0, y0), (x1, y1) in pairwise(points):
        if x0 <= x <= x1:
            return y0 if x1 == x0 else y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    return None


def fer_curves(path: Path) -> dict[int, list[tuple[float, float]]]:
    curves: dict[int, list[tuple[float, float]]] = defaultdict(list)
    with path.open(encoding="utf-8") as f:
        for row in csv.DictReader(f):
            if row["channel"] == "awgn":
                curves[int(row["mode"])].append((float(row["snr_3k_db"]), float(row["fer"])))
    return curves


def goodput_curves(path: Path) -> dict[str, list[tuple[float, float]]]:
    curves: dict[str, list[tuple[float, float]]] = defaultdict(list)
    with path.open(encoding="utf-8") as f:
        for row in csv.DictReader(f):
            if row["ramp"]:
                continue  # fade ramps are a different experiment
            curves[row["channel"]].append((float(row["snr_db"]), float(row["goodput_bps"])))
    # several trials per point: average them
    averaged: dict[str, list[tuple[float, float]]] = {}
    for channel, points in curves.items():
        by_snr: dict[float, list[float]] = defaultdict(list)
        for snr, goodput in points:
            by_snr[snr].append(goodput)
        averaged[channel] = sorted((snr, statistics.mean(g)) for snr, g in by_snr.items())
    return averaged


def report(sidecar: Path, channel: str | None, baselines: Path) -> int:
    document = json.loads(sidecar.read_text(encoding="utf-8"))
    if document.get("format") != "aether-hf-session/1":
        sys.exit(f"{sidecar}: not a session sidecar")
    frames = document.get("frames", [])
    data = [f for f in frames if f["kind"] == "data"]
    session = document.get("session", {})
    counters = document.get("counters") or {}
    seconds = float(document["audio"]["seconds"])
    print(
        f"{sidecar.name}: {session.get('callsign')} with {session.get('remote')}, "
        f"{seconds:.0f} s, notes: {session.get('notes') or '-'}"
    )

    if not data:
        print("no data frames were heard; nothing to compare")
        return 0

    fer = fer_curves(baselines / "phy_fer_awgn14.csv")
    print(
        f"\n{'mode':>4} {'frames':>6} {'failed':>6} {'SNR dB':>7} {'FER meas':>9} {'FER AWGN':>9}"
    )
    by_mode: dict[int, list[dict[str, float]]] = defaultdict(list)
    for frame in data:
        by_mode[int(frame["mode"])].append(frame)
    snrs = [float(f["snr_3k_db"]) for f in data]
    for mode in sorted(by_mode):
        heard = by_mode[mode]
        failed = sum(1 for f in heard if not f["decoded"])
        snr = statistics.mean(float(f["snr_3k_db"]) for f in heard)
        measured = failed / len(heard)
        predicted = interpolate(fer.get(mode, []), snr)
        shown = f"{predicted:9.3f}" if predicted is not None else f"{'-':>9}"
        print(f"{mode:>4} {len(heard):>6} {failed:>6} {snr:>7.1f} {measured:>9.3f} {shown}")

    delivered = counters.get("bytes_delivered")
    mean_snr = statistics.mean(snrs)
    print(f"\nmean SNR over data frames: {mean_snr:.1f} dB (3 kHz reference)")
    if delivered is None or seconds <= 0:
        print("no delivered-bytes counter in the sidecar; goodput cannot be compared")
        return 0
    measured_bps = 8 * float(delivered) / seconds
    # link bytes — after compression, as the frames carried them — which is what the
    # simulator's goodput counts too; the application's bytes are a different number
    print(
        f"goodput measured: {measured_bps:.0f} bit/s ({delivered} link bytes in {seconds:.0f} s, "
        f"connect to disconnect)"
    )
    goodput = goodput_curves(baselines / "link_throughput.csv")
    classes = [channel] if channel else list(CLASSES)
    worst_within = None
    for name in classes:
        predicted = interpolate(goodput.get(name, []), mean_snr)
        if predicted is None:
            print(f"  {name:>8}: no baseline")
            continue
        ratio = measured_bps / predicted if predicted else float("inf")
        verdict = "within 20 %" if 0.8 <= ratio <= 1.2 else f"{(ratio - 1) * 100:+.0f} %"
        print(
            f"  {name:>8}: predicted {predicted:6.0f} bit/s  measured/predicted {ratio:5.2f}  {verdict}"
        )
        if channel:
            worst_within = 0.8 <= ratio <= 1.2
    if channel and worst_within is False:
        print("\nthe air and the simulator disagree by more than 20 % for this channel class")
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("sidecar", type=Path)
    parser.add_argument("--channel", choices=CLASSES, help="the channel class the air was")
    parser.add_argument("--baselines", type=Path, default=ROOT / "bench" / "baselines")
    args = parser.parse_args()
    return report(args.sidecar, args.channel, args.baselines)


if __name__ == "__main__":
    sys.exit(main())
