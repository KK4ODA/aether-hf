"""How reliably a DATA frame's burst countdown is read (ADR-0041).

    python tools/bench_follows.py [--frames 100] [--jobs 4]

Every ordinary DATA frame says how many more frames of its burst follow it (0–3) by turning
its mode chips a quarter turn a frame. A receiver that reads fewer than were sent may answer
over the rest of the burst — what the countdown is there to prevent — and one that reads more
waits a frame or two longer than it had to. For each air, channel and SNR the frame goes
through the channel, the detector and the demodulator as a station would take it, and the
lines say how many of the frames detected had their mode and RV read right, how many their
countdown, how many read short (the unsafe way) and how many decoded.
"""

from __future__ import annotations

import argparse
import sys
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.waveform import WAVEFORMS, Bandwidth

CHANNELS = ("awgn", "good", "moderate", "poor")
SNRS = (-6, -3, 0, 3, 6)


def case(job: tuple[int, str, int, int, int, bool]) -> str:
    bw, channel, snr, frames, seed, countdown = job
    params = WAVEFORMS[Bandwidth(bw)]
    modem = Modem(params)
    air = modem.air
    first = next(r.mode for r in air.ladder if r.mode is not None)
    layout = air.long
    codec = modem.codec(first, layout)
    rng = np.random.default_rng(seed)
    detected = right = counted = short = decoded = 0
    for _ in range(frames):
        drawn = int(rng.integers(0, 4))  # drawn either way, so both runs see the same frames
        follows = drawn if countdown else 0
        rv = int(rng.integers(0, 4))
        payload = bytes(rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8))
        burst = modem.tx.baseband(
            FrameHeader(FrameType.DATA, first.index, rv, follows), layout, codec.encode(payload, rv)
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
        detected += 1
        if frame.mode != first.index or frame.rv != rv:
            continue
        right += 1
        counted += frame.follows == follows
        short += frame.follows < follows
        out, _ = codec.decode(frame.symbols, frame.noise_var, rv=rv)
        decoded += out == payload
    return (
        f"{bw:<5d}{channel:9s}{snr:+3d} dB  detected {detected:3d}/{frames}  mode+rv {right:3d}"
        f"  countdown {counted:3d}  read short {short:2d}  decoded at rv {decoded:3d}"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--frames", type=int, default=100)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument(
        "--no-countdown",
        action="store_true",
        help="send every frame unturned, with the same draws: the decode rate to compare",
    )
    args = ap.parse_args()
    jobs = []
    seed = 41
    for bw in (2300, 500):
        for channel in CHANNELS:
            for snr in SNRS:
                seed += 1
                jobs.append((bw, channel, snr, args.frames, seed, not args.no_countdown))
    with Pool(args.jobs) as pool:
        for line in pool.imap(case, jobs):
            print(line, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
