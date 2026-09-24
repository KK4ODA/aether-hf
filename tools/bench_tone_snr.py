"""What the tone floor's SNR estimate reads against the channel's SNR, per channel class.

    python tools/bench_tone_snr.py [--trials 20] [--out bench/baselines/tone_snr_reading.csv]

The tone floor measures SNR by energy (ADR-0013): the sync symbols' tone energy over the
median of the bins that hold no tone. That is exact at low SNR, where the floor is used, and
reads low on a strong path — the glide between two tones spills a little of every symbol into
every bin, some 43 dB down, which caps the reading near +17 dB on AWGN; on a dispersive path
the echo's spill into the next symbol counts as noise too, and the reading stops at a few
decibels whatever the SNR. Since calls, probes and beacons start on the floor (ADR-0016), a
session's first measurement is this one: the link bench's ``--floor-cap`` takes each class's
ceiling from here. Genie timing and carrier offset, so the estimate alone is measured; the
frames are the ones a call and its answer use.
"""

from __future__ import annotations

import argparse
import csv
import sys
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import WIDE
from aether_model.phy import tone as T

CHANNELS = ("awgn", "good", "moderate", "poor")
SNRS = range(-10, 45, 5)


def run(job: tuple[str, str, int, int]) -> dict[str, object]:
    name, channel, snr, trials = job
    kind = next(k for k in (WIDE.tone_control, *WIDE.tone_data) if k.name == name)
    rng = np.random.default_rng([CHANNELS.index(channel), snr + 100, len(name)])
    reads = []
    for _ in range(trials):
        payload = bytes(rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))
        x = T.burst(kind, payload, 0)
        buf = np.concatenate((np.zeros(2000, complex), x, np.zeros(2000, complex)))
        ch = make_channel(
            channel,
            snr_db=float(snr),
            fs=kind.num.fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=0.0,
        )
        reads.append(T.demodulate(ch.process(buf), kind, 0, 2000, 0.0).snr_db)
    return {
        "frame": name,
        "channel": channel,
        "snr_db": snr,
        "trials": trials,
        "read_median_db": round(float(np.median(reads)), 2),
        "read_mean_db": round(float(np.mean(reads)), 2),
        "read_p10_db": round(float(np.percentile(reads, 10)), 2),
        "read_p90_db": round(float(np.percentile(reads, 90)), 2),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--trials", type=int, default=20)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", default="bench/baselines/tone_snr_reading.csv")
    args = ap.parse_args()
    jobs = [
        (kind.name, channel, snr, args.trials)
        for kind in (WIDE.tone_data[0], WIDE.tone_control)
        for channel in CHANNELS
        for snr in SNRS
    ]
    rows: list[dict[str, object]] = []
    with Pool(args.jobs) as pool:
        for row in pool.imap(run, jobs):
            rows.append(row)
            print(
                f"{row['frame']:13s} {row['channel']:8s} {row['snr_db']:+4d} dB"
                f"  reads {row['read_median_db']:+6.1f} (p10 {row['read_p10_db']:+6.1f},"
                f" p90 {row['read_p90_db']:+6.1f})",
                flush=True,
            )
    with Path(args.out).open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
