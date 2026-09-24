"""Where the tone floor breaks (P9-8, ADR-0013): acquisition, decode and genie decode per kind.

    python tools/bench_tone.py [--frames 100] [--channels awgn,good,moderate,poor]
                               [--kinds tone-control,tone-24,...,tone100-153] [--jobs 3]
                               [--out bench/baselines/tone_floor.csv]

For every tone-floor kind, ``--frames`` frames a point, three things are counted, as
``bench_floor.py`` counts them for the OFDM frames: frames the detector found with the right
kind and redundancy version within a quarter symbol of the truth; frames that decoded through
the detector; and frames that decoded with *genie* timing and carrier offset. Each frame has a
random start and a carrier offset uniform in ±100 Hz. The detector is the 2 300 Hz air's, which
looks for every kind — the floor's and the fast ones of ADR-0014 — so a kind is also tested
against being taken for another.

The SNR is the OFDM reference (3 kHz, against an OFDM frame's average power at the same
transmit level), and the tone frames go out :data:`~aether_model.phy.tone.TONE_GAIN_DB`
above it — equal peak power — so these curves read directly against ``floor_500.csv`` and
``phy_fer*.csv``: the comparison ADR-0013's gate is decided on.
"""

from __future__ import annotations

import argparse
import csv
import sys
import time
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import WIDE
from aether_model.phy import tone as T

RANGES = {
    "awgn": range(-26, -13, 1),
    "good": range(-26, -3, 2),
    "moderate": range(-26, -3, 2),
    "poor": range(-26, -3, 2),
}
"""The floor's own kinds' SNRs; a fast kind's run :data:`FAST_SHIFT_DB` higher."""
FAST_SHIFT_DB = 6
KINDS = {k.name: k for k in (WIDE.tone_control, *WIDE.tone_data)}


def snrs(name: str, channel: str) -> range:
    r = RANGES[channel]
    shift = FAST_SHIFT_DB if KINDS[name].speed > 1 else 0
    return range(r.start + shift, r.stop + shift, r.step)


def run(job: tuple[str, str, int, int, int]) -> dict[str, object]:
    name, channel, snr, frames, seed = job
    kind = KINDS[name]
    det = T.ToneDetector(tuple(KINDS.values()))
    rng = np.random.default_rng(seed)
    acquired = decoded = genie = 0
    tail = 2000
    for _ in range(frames):
        payload = bytes(rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))
        x = T.burst(kind, payload, 0)
        lead = int(rng.integers(1000, 4000))
        buf = np.concatenate((np.zeros(lead, complex), x, np.zeros(tail, complex)))
        cfo = float(rng.uniform(-100, 100))
        ch = make_channel(
            channel,
            snr_db=float(snr),
            fs=kind.num.fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=cfo,
        )
        y = ch.process(buf)
        syncs = det.detect(y, max_frames=2)
        hit = [s for s in syncs if s.kind == kind and s.rv == 0 and abs(s.start - lead) <= det.hop]
        if hit:
            acquired += 1
            s = hit[0]
            out, _ = T.demodulate(y, kind, 0, s.start, s.cfo_hz).decode()
            decoded += out == payload
        out, _ = T.demodulate(y, kind, 0, lead, cfo).decode()
        genie += out == payload
    return {
        "frame": name,
        "payload_bytes": kind.payload_bytes,
        "net_bps": round(kind.net_bps, 1),
        "channel": channel,
        "snr_3k_db": snr,
        "frames": frames,
        "acquired": acquired,
        "decoded": decoded,
        "genie_decoded": genie,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--frames", type=int, default=100)
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--kinds", default=",".join(KINDS))
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", default="bench/baselines/tone_floor.csv")
    args = ap.parse_args()

    jobs = [
        (name, channel, snr, args.frames, args.seed * 1_000_003 + 7919 * i)
        for i, (name, channel, snr) in enumerate(
            (n, c, s)
            for n in args.kinds.split(",")
            for c in args.channels.split(",")
            for s in snrs(n, c)
        )
    ]
    t0 = time.time()
    rows = []
    with Pool(args.jobs) as pool:
        for row in pool.imap(run, jobs):
            rows.append(row)
            print(
                f"{row['frame']:13s} {row['channel']:8s} {row['snr_3k_db']:+4d} dB  "
                f"acq {row['acquired']:3d}/{row['frames']}  dec {row['decoded']:3d}  "
                f"genie {row['genie_decoded']:3d}   [{time.time() - t0:5.0f} s]",
                flush=True,
            )
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    print("wrote", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
