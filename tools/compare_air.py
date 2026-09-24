"""Measured against predicted: what a recorded session did, next to what the simulator says.

    python tools/compare_air.py field/sessions/<name>.json [--channel awgn|good|moderate|poor]
                                [--baselines bench/baselines] [--no-sim]

Field validation (roadmap Phase 6) ends with "simulator-vs-air throughput within 20 %", and
this is the comparison. From the sidecar it takes what the receiving station measured —
the frames it heard, their modes and SNRs, which decoded, and the link bytes delivered over
the session — and puts beside each number what the simulator says for the same SNR:

* the AWGN frame error rate per mode, from the committed curve (`phy_fer_awgn14.csv`);
* the link goodput the reference model's discrete-event simulator delivers for **this
  session's size** at this SNR on this channel class, with a real station's latency
  (a quarter-second playback backlog and the keying lead, which the benchmark grid does
  not carry) — the like-for-like number, and the one the 20 % verdict is against;
* the committed asymptotic goodput curve (`link_throughput.csv`, 16 kB transfers), for
  scale: a short session sits below it because connect, the rate ramp and the disconnect
  are a larger share of it.

The channel class is the operator's call (`--channel`); without it every class is shown, and
the one the air was closest to is the one to write down. A session that disagrees with the
simulator by more than 20 % is not a failure of either — it is the number the roadmap says
to recalibrate on, once there are enough of them.
"""

from __future__ import annotations

import argparse
import csv
import json
import random
import statistics
import sys
from collections import defaultdict
from dataclasses import replace
from itertools import pairwise
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from aether_model.frame.modes import NARROW, WIDE
from aether_model.link.engine import LinkEngine
from aether_model.link.harness import phy_timing
from aether_model.link.sim import TwoStationSim
from aether_model.waveform import WIDE_2300
from bench_link import channel_thresholds
from field_ingest import FORMATS, sidecar_rung

ROOT = Path(__file__).resolve().parents[1]
CLASSES = ("awgn", "good", "moderate", "poor")

# What a real station adds that the benchmark grid does not: the daemon keeps a quarter of
# a second of playback queued ahead of the sound card and keys 0.1 s before a burst
# (`PhyTiming.tx_latency_s`), and the samples take about as long again to come back
# through the other station's capture and receiver.
REAL_TX_LATENCY_S = 0.4
REAL_PROPAGATION_S = 0.45


def predict_session(
    link_bytes: int, snr_db: float, channel: str, fer_csv: Path, trials: int = 3
) -> float:
    """Goodput the reference model's simulator gives a session of this size, with a real
    station's latency: connect, one transfer, disconnect, averaged over a few seeds."""
    # the wide air's ladder as the model's PHY reports it, with a real station's latency
    timing = replace(phy_timing(WIDE_2300), tx_latency_s=REAL_TX_LATENCY_S)
    thresholds = None if channel == "awgn" else channel_thresholds(fer_csv).get(channel)
    payload = bytes(random.Random(7).getrandbits(8) for _ in range(link_bytes))
    rates = []
    for trial in range(trials):
        a = LinkEngine("W4ODA", timing, None, seed=100 + trial)
        b = LinkEngine("KK4XYZ", timing, None, seed=200 + trial)
        sim = TwoStationSim(
            a,
            b,
            snr_db=snr_db,
            seed=300 + trial,
            prop_s=REAL_PROPAGATION_S,
            thresholds=thresholds,
        )
        a.connect("KK4XYZ")
        a.send(payload)
        a.disconnect()
        seconds = sim.run(until=80.0 * link_bytes / 1000.0 + 400.0)
        rates.append(8 * len(sim.delivered(1)) / seconds if seconds > 0 else 0.0)
    return statistics.mean(rates)


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


def report(sidecar: Path, channel: str | None, baselines: Path, simulate: bool) -> int:
    document = json.loads(sidecar.read_text(encoding="utf-8"))
    if document.get("format") not in FORMATS:
        sys.exit(f"{sidecar}: not a session sidecar")
    frames = document.get("frames", [])
    data = [f for f in frames if f["kind"] == "data"]
    session = document.get("session", {})
    counters = document.get("counters") or {}
    seconds = float(document["audio"]["seconds"])
    frequency = session.get("frequency_hz")
    where = f"{frequency / 1e6:.4f} MHz" if frequency else "frequency not recorded"
    print(
        f"{sidecar.name}: {session.get('callsign')} with {session.get('remote')}, "
        f"{seconds:.0f} s, {where}, notes: {session.get('notes') or '-'}"
    )

    if not data:
        print("no data frames were heard; nothing to compare")
        return 0

    fer = fer_curves(baselines / "phy_fer_awgn14.csv")
    print(
        f"\n{'mode':>4} {'frames':>6} {'failed':>6} {'SNR dB':>7} {'FER meas':>9} {'FER AWGN':>9}"
    )
    # the sidecar's mode numbers onto the ladder (older formats numbered it otherwise), and
    # a rung onto the OFDM mode the curve is kept by; a tone kind's curve is not in it
    air = NARROW if session.get("bandwidth_hz") == 500 else WIDE
    by_mode: dict[int, list[dict[str, float]]] = defaultdict(list)
    for frame in data:
        rung = sidecar_rung(document, int(frame["mode"]))
        if rung is not None:
            by_mode[rung].append(frame)
    snrs = [float(f["snr_3k_db"]) for f in data]
    for mode in sorted(by_mode):
        heard = by_mode[mode]
        failed = sum(1 for f in heard if not f["decoded"])
        snr = statistics.mean(float(f["snr_3k_db"]) for f in heard)
        measured = failed / len(heard)
        ofdm = air.ladder[mode].mode if mode < air.n_rungs else None
        curve = fer.get(ofdm.index, []) if ofdm is not None and air is WIDE else []
        expected = interpolate(curve, snr)
        shown = f"{expected:9.3f}" if expected is not None else f"{'-':>9}"
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
    within = None
    print(f"{'class':>10} {'same session':>14} {'ratio':>6}  {'16 kB curve':>12}")
    for name in classes:
        asymptotic = interpolate(goodput.get(name, []), mean_snr)
        shown_curve = f"{asymptotic:9.0f} b/s" if asymptotic is not None else f"{'-':>12}"
        if simulate:
            predicted: float | None = predict_session(
                int(delivered), mean_snr, name, baselines / "phy_fer.csv"
            )
        else:
            predicted = asymptotic
        if predicted is None:
            print(f"{name:>10} {'no baseline':>14}")
            continue
        ratio = measured_bps / predicted if predicted else float("inf")
        verdict = "within 20 %" if 0.8 <= ratio <= 1.2 else f"{(ratio - 1) * 100:+.0f} %"
        print(f"{name:>10} {predicted:10.0f} b/s {ratio:6.2f}  {shown_curve}  {verdict}")
        if channel:
            within = 0.8 <= ratio <= 1.2
    if channel and within is False:
        print("\nthe air and the simulator disagree by more than 20 % for this channel class")
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("sidecar", type=Path)
    parser.add_argument("--channel", choices=CLASSES, help="the channel class the air was")
    parser.add_argument("--baselines", type=Path, default=ROOT / "bench" / "baselines")
    parser.add_argument(
        "--no-sim",
        action="store_true",
        help="compare with the asymptotic curve only, without running the simulator",
    )
    args = parser.parse_args()
    return report(args.sidecar, args.channel, args.baselines, not args.no_sim)


if __name__ == "__main__":
    sys.exit(main())
