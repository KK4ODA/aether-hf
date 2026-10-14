"""How often a data frame's count is read wrong, by whether the frame decoded on its own
(10-step-3.md §6.2).

    uv run python research/link_arch/count_reading.py [--frames 100] [--jobs 4]

A frame's count rides in its chips, a quarter turn a frame, outside the codeword. On the
scenario harness a frame that HARQ rescued (its own copy did not decode) had its count read
short, and the slave answered over the frames still to come. Every OFDM data frame here goes
through the channel, the detector and the demodulator as a station takes it; among those whose
mode and RV read right, the counts are tallied for the frames that decoded alone (``alone``) and
those that did not (``weak``). The result is ``results/count_reading.txt``.
"""

from __future__ import annotations

import argparse
import sys
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "model"))

from aether_model.channel import make_channel
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.waveform import WAVEFORMS, Bandwidth


def case(job: tuple[int, int, str, int, int, int]) -> tuple[int, int, str, int, dict[str, int]]:
    bw, rung, channel, snr, frames, seed = job
    params = WAVEFORMS[Bandwidth(bw)]
    modem = Modem(params)
    air = modem.air
    mode = air.ladder[rung].mode
    layout = air.long
    codec = modem.codec(mode, layout)
    rng = np.random.default_rng(seed)
    n = dict(
        detected=0,
        right=0,
        alone=0,
        alone_wrong=0,
        alone_short=0,
        weak=0,
        weak_wrong=0,
        weak_short=0,
    )
    for _ in range(frames):
        follows = int(rng.integers(0, 4))
        rv = int(rng.choice([0, 2, 3]))
        payload = bytes(rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8))
        burst = modem.tx.baseband(
            FrameHeader(FrameType.DATA, mode.index, rv, follows), layout, codec.encode(payload, rv)
        )
        lead = int(rng.integers(1000, 3000))
        buf = np.concatenate((np.zeros(lead, complex), burst, np.zeros(4000, complex)))
        y = make_channel(
            channel,
            snr_db=float(snr),
            fs=params.fs_baseband,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-50, 50)),
        ).process(buf)
        y = modem.detector.condition(y)
        syncs = modem.detector.detect(y, max_frames=1)
        if not syncs or syncs[0].header.frame_type is not FrameType.DATA:
            continue
        try:
            frame = modem.demodulate(y, syncs[0])
        except ValueError:
            continue
        n["detected"] += 1
        if frame.mode != mode.index or frame.rv != rv:
            continue
        n["right"] += 1
        out, _ = codec.decode(frame.symbols, frame.noise_var, rv=rv)
        key = "alone" if out == payload else "weak"
        n[key] += 1
        n[key + "_wrong"] += frame.follows != follows
        n[key + "_short"] += frame.follows < follows
    return (bw, rung, channel, snr, n)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--frames", type=int, default=200)
    ap.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    jobs = []
    seed = 7000
    for bw, rungs in ((500, (4, 5, 7)), (2300, (7, 9))):
        for rung in rungs:
            for channel in ("awgn", "nvis", "moderate", "poor"):
                for snr in (-6, -3, 0, 3):
                    seed += 1
                    jobs.append((bw, rung, channel, snr, args.frames, seed))
    tot: dict[str, int] = {}
    with Pool(args.jobs) as pool:
        for bw, rung, channel, snr, n in pool.imap(case, jobs):
            for k, v in n.items():
                tot[k] = tot.get(k, 0) + v
            print(f"{bw:5d} r{rung:<2d} {channel:9s} {snr:+3d}  {n}", flush=True)
    print("TOTAL", tot)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
