"""How reliably a tone frame's count is read, and what reading it costs (ADR-0068).

    python tools/bench_tone_follows.py [--frames 60] [--jobs 4] [--no-count]
                                       [--out bench/baselines/tone_follows.csv]
    python tools/bench_tone_follows.py --noise 30 [--bandwidth 500]

Every tone DATA frame says how many frames of its burst follow it (0–3) in its middle sync
block, which carries the kind's pattern for redundancy version ``rv + count``; the start and end
blocks keep the frame's own. A receiver that reads the count wrong answers at the wrong time —
early, over the rest of the burst, or late, into the sender's repeat — so a count read without
a clear margin states nothing (``ToneSync.follows`` is ``None``) and the burst closes
unanswered instead.

For each data kind on its own air, each channel and SNR, ``--frames`` frames with a random
redundancy version and a random count go through the channel and the air's detector, as
``bench_tone.py`` sends them; the row says how many were acquired with the right kind and RV,
how many of those sent at RV 0 decoded, and of those acquired how many had their count read right, not stated, or
read wrong (and of those, short — the unsafe way). ``--no-count`` sends the same draws with
nothing following: the detector's sensitivity to compare, since the middle block is now read
as the best of the kind's four patterns.

``--noise MINUTES`` instead runs the air's detector over that much white noise and counts the
frames it confirms and the frames a stream announces: the false alarms the best-of-four middle
block adds, to compare with the same run on the tree before it.
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
from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.phy import tone as T

CHANNELS = ("awgn", "good", "moderate", "poor")
RANGES = {
    "awgn": range(-24, -13, 2),
    "good": range(-22, -3, 3),
    "moderate": range(-22, -3, 3),
    "poor": range(-22, -3, 3),
}
"""The floor's own kinds' SNRs (OFDM reference); a fast or middle kind's run
:data:`SHIFT_DB` higher, as in ``bench_tone.py``."""
SHIFT_DB = 6
AIRS: dict[str, AirInterface] = {k.name: air for air in (NARROW, WIDE) for k in air.tone_data}
"""Each data kind's air — the 2 300 Hz air's for the kinds both airs share."""
KINDS = {k.name: k for air in (WIDE, NARROW) for k in air.tone_data}


def snrs(name: str, channel: str) -> range:
    r = RANGES[channel]
    shift = SHIFT_DB if name not in ("tone-24", "tone-36") else 0
    return range(r.start + shift, r.stop + shift, r.step)


def run(job: tuple[str, str, int, int, int, bool]) -> dict[str, object]:
    name, channel, snr, frames, seed, count = job
    kind = KINDS[name]
    air = AIRS[name]
    det = T.ToneDetector((air.tone_control, *air.tone_data))
    rng = np.random.default_rng(seed)
    acquired = sent0 = decoded = right = unstated = wrong = short = 0
    for _ in range(frames):
        rv = int(rng.integers(0, len(kind.patterns)))
        drawn = int(rng.integers(0, 4))  # drawn either way: both runs see the same frames
        follows = drawn if count else 0
        payload = bytes(rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))
        sent0 += rv == 0
        x = T.burst(kind, payload, rv, follows)
        lead = int(rng.integers(1000, 4000))
        buf = np.concatenate((np.zeros(lead, complex), x, np.zeros(2000, complex)))
        y = make_channel(
            channel,
            snr_db=float(snr),
            fs=kind.num.fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-100, 100)),
        ).process(buf)
        hits = [
            s
            for s in det.detect(y, max_frames=2)
            if s.kind == kind and s.rv == rv and abs(s.start - lead) <= det.hop
        ]
        if not hits:
            continue
        s = hits[0]
        acquired += 1
        if s.follows is None:
            unstated += 1
        elif s.follows == follows:
            right += 1
        else:
            wrong += 1
            short += s.follows < follows
        if rv == 0:
            # a lone copy at RV 1 or 2 never decodes, whatever its count (ADR-0043)
            out, _ = T.demodulate(y, kind, rv, s.start, s.cfo_hz, middle=s.middle).decode()
            decoded += out == payload
    return {
        "frame": name,
        "bandwidth_hz": air.params.bandwidth.value,
        "channel": channel,
        "snr_3k_db": snr,
        "counts": int(count),
        "frames": frames,
        "acquired": acquired,
        "sent_rv0": sent0,
        "decoded_rv0": decoded,
        "count_right": right,
        "count_unstated": unstated,
        "count_wrong": wrong,
        "count_short": short,
    }


def noise(job: tuple[int, float, int]) -> tuple[int, int, int]:
    """Confirmed frames and announcements on ``minutes`` of white noise at one air."""
    bw, minutes, seed = job
    air = NARROW if bw == 500 else WIDE
    det = T.ToneDetector((air.tone_control, *air.tone_data))
    stream = T.ToneStream(det)
    rng = np.random.default_rng(seed)
    fs = air.tone_control.num.fs
    block = int(fs * 2)
    keep = max(k.samples for k in (air.tone_control, *air.tone_data)) + block
    buf = np.zeros(0, complex)
    abs0 = 0
    found = announced = 0
    seen: set[int] = set()
    for _ in range(int(minutes * 60 * fs / block)):
        chunk = (rng.standard_normal(block) + 1j * rng.standard_normal(block)) / np.sqrt(2)
        buf = np.concatenate((buf, chunk))
        found += len(stream.feed(buf, abs0))
        for a in stream.arriving:
            if a.start not in seen:
                seen.add(a.start)
                announced += 1
        if len(buf) > keep:
            cut = len(buf) - keep
            buf = buf[cut:]
            abs0 += cut
    return bw, found, announced


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--frames", type=int, default=60)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--kinds", default=",".join(KINDS))
    ap.add_argument("--channels", default=",".join(CHANNELS))
    ap.add_argument("--no-count", action="store_true", help="the same draws, nothing following")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default="")
    ap.add_argument("--noise", type=float, default=0.0, help="minutes of noise per air instead")
    ap.add_argument("--bandwidth", default="2300,500", help="the airs of a noise run")
    args = ap.parse_args()

    if args.noise:
        jobs = []
        for i, bw in enumerate(int(b) for b in args.bandwidth.split(",")):
            for part in range(args.jobs):
                jobs.append((bw, args.noise / args.jobs, args.seed + 100 * i + part))
        totals: dict[int, list[int]] = {}
        with Pool(args.jobs) as pool:
            for bw, found, announced in pool.imap(noise, jobs):
                t = totals.setdefault(bw, [0, 0])
                t[0] += found
                t[1] += announced
        for bw, (found, announced) in totals.items():
            print(f"{bw} Hz: {args.noise:g} min of noise: {found} frames, {announced} announced")
        return 0

    jobs = []
    seed = args.seed
    for name in args.kinds.split(","):
        for channel in args.channels.split(","):
            for snr in snrs(name, channel):
                seed += 1
                jobs.append((name, channel, snr, args.frames, seed, not args.no_count))
    rows = []
    with Pool(args.jobs) as pool:
        for row in pool.imap(run, jobs):
            rows.append(row)
            print(
                f"{row['bandwidth_hz']:<5}{row['frame']:14s}{row['channel']:9s}"
                f"{row['snr_3k_db']:+4d} dB  acquired {row['acquired']:3d}/{row['frames']}"
                f"  decoded {row['decoded_rv0']:3d}/{row['sent_rv0']} at rv 0"
                f"  count right {row['count_right']:3d}"
                f"  unstated {row['count_unstated']:2d}  wrong {row['count_wrong']:2d}"
                f" (short {row['count_short']})",
                flush=True,
            )
    if args.out:
        with open(args.out, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
