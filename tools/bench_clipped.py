"""What a frame survives of its own start: decodes when the receiver was deaf for its first part.

    python tools/bench_clipped.py [--frames 30] [--jobs 4]

A receiver that comes back from transmitting late, or a peer still keyed by its VOX hold
(ADR-0036), never hears the start of the frame that answers it. The receive audio is the
radio's silence until then, so the frame's first ``clip`` seconds are zeroed after the
channel and the frame goes through the detector and decoder as the station's receiver
would take it. Covers the tone floor's control and data kinds and the ordinary family's
control frame (the acknowledgements and polls of a session at an OFDM rung) on both airs
(ADR-0037).
"""

from __future__ import annotations

import argparse
import sys
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import NARROW, air_interface
from aether_model.phy import tone as T
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.waveform import WAVEFORMS, Bandwidth

CLIPS = (0.0, 0.1, 0.2, 0.3, 0.5)
TONE_POINTS = (("awgn", -10), ("good", -6), ("awgn", 0))
OFDM_POINTS = (("awgn", 0), ("good", 4))


def tone_case(job: tuple[str, str, int, float, int, int]) -> str:
    kname, channel, snr, clip, frames, seed = job
    kinds = (NARROW.tone_control, *NARROW.tone_data)
    kind = next(k for k in kinds if k.name == kname)
    det = T.ToneDetector(kinds)
    rng = np.random.default_rng(seed)
    ok = 0
    for _ in range(frames):
        payload = bytes(rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))
        lead = int(rng.integers(1000, 3000))
        buf = np.concatenate(
            (np.zeros(lead, complex), T.burst(kind, payload, 0), np.zeros(2000, complex))
        )
        y = make_channel(
            channel,
            snr_db=float(snr),
            fs=kind.num.fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-50, 50)),
        ).process(buf)
        y[: lead + int(clip * kind.num.fs)] = 0
        for s in det.detect(y, max_frames=2):
            if s.kind == kind and s.rv == 0:
                out, _ = T.demodulate(y, kind, 0, s.start, s.cfo_hz).decode()
                if out == payload:
                    ok += 1
                    break
    return f"500 {kname:24s} {channel:5s} {snr:+3d} dB  first {clip * 1000:3.0f} ms lost: {ok}/{frames}"


def ofdm_case(job: tuple[int, str, int, float, int, int]) -> str:
    bw, channel, snr, clip, frames, seed = job
    params = WAVEFORMS[Bandwidth(bw)]
    air = air_interface(params)
    modem = Modem(params)
    fs = params.fs_baseband
    codec = modem.codec(air.control_mode, air.short)
    rng = np.random.default_rng(seed)
    ok = 0
    for _ in range(frames):
        payload = bytes(rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8))
        burst = modem.tx.baseband(
            FrameHeader(FrameType.CONTROL), air.short, codec.encode(payload, 0)
        )
        lead = 2000
        buf = np.concatenate((np.zeros(lead, complex), burst, np.zeros(2000, complex)))
        y = make_channel(
            channel,
            snr_db=float(snr),
            fs=fs,
            seed=int(rng.integers(0, 2**31)),
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-50, 50)),
        ).process(buf)
        y[: lead + int(clip * fs)] = 0
        y = modem.detector.condition(y)
        for s in modem.detector.detect(y, max_frames=1):
            try:
                frame = modem.demodulate(y, s)
                out, _ = codec.decode(frame.symbols, frame.noise_var, rv=0)
            except ValueError:
                continue
            ok += out == payload
    name = f"OFDM control ({air.short.duration_s:.2f} s)"
    return f"{bw:<4d}{name:24s} {channel:5s} {snr:+3d} dB  first {clip * 1000:3.0f} ms lost: {ok}/{frames}"


def case(job: tuple[str, tuple[object, ...]]) -> str:
    family, args = job
    if family == "tone":
        return tone_case(args)  # type: ignore[arg-type]
    return ofdm_case(args)  # type: ignore[arg-type]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    jobs: list[tuple[str, tuple[object, ...]]] = []
    seed = 11
    for kname in (NARROW.tone_control.name, NARROW.tone_data[1].name):
        for channel, snr in TONE_POINTS:
            for clip in CLIPS:
                seed += 1
                jobs.append(("tone", (kname, channel, snr, clip, args.frames, seed)))
    for bw in (500, 2300):
        for channel, snr in OFDM_POINTS:
            for clip in CLIPS:
                seed += 1
                jobs.append(("ofdm", (bw, channel, snr, clip, args.frames, seed)))
    with Pool(args.jobs) as pool:
        for line in pool.imap(case, jobs):
            print(line, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
