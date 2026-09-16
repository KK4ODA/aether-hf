"""Where the floor breaks: acquisition, decode and genie-timing decode per frame (ADR-0009).

    python tools/bench_floor.py [--bandwidth 500] [--frames 20] [--channels awgn,good,poor]
                                [--out bench/baselines/floor_500.csv]

For every mode of the air (on the layout it goes out on) and for the two control frames
(ordinary and floor), twenty frames a point, three things are counted: frames the detector
placed within half a symbol of the truth, frames that decoded through the detector, and
frames that decoded with *genie* timing — the true start, the detector's own CFO from
there. Genie against detected separates the code-and-modulation floor from the receiver's.
``bench_phy.py`` gives the FER curves the rate table is built from; this tool says *why* a
curve stops where it does.
"""

from __future__ import annotations

import argparse
import csv
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import air_interface
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.phy.sync import FrameSync
from aether_model.waveform import WAVEFORMS, Bandwidth

RANGES = {
    "awgn": range(-18, -1, 1),
    "good": range(-16, 5, 2),
    "moderate": range(-16, 5, 2),
    "poor": range(-16, 5, 2),
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bandwidth", type=int, default=500, choices=(2300, 500))
    ap.add_argument("--frames", type=int, default=20)
    ap.add_argument("--channels", default="awgn,good,poor")
    ap.add_argument("--modes", default=None, help="mode indices; default: the three slowest")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    params = WAVEFORMS[Bandwidth(args.bandwidth)]
    air = air_interface(params)
    modem = Modem(params)
    fs = params.fs_baseband
    lead, tail = 2000, 2000
    indices = (
        [int(m) for m in args.modes.split(",")] if args.modes else [m.index for m in air.modes[:3]]
    )
    cases: list[tuple[str, object, FrameHeader, object]] = []
    for i in indices:
        mode = air.modes[i]
        layout = air.data_layout(i)
        cases.append(
            (f"mode {i} {mode.name} {layout.name}", mode, FrameHeader(FrameType.DATA, i), layout)
        )
    cases.append(("control short", air.control_mode, FrameHeader(FrameType.CONTROL), air.short))
    if air.floor_short is not None:
        cases.append(
            (
                "control floor",
                air.floor_control_mode,
                FrameHeader(FrameType.CONTROL),
                air.floor_short,
            )
        )
    rng = np.random.default_rng(args.seed)
    rows = []
    t0 = time.time()
    for label, mode, header, layout in cases:
        codec = modem.codec(mode, layout)  # type: ignore[arg-type]
        n = codec.payload_bytes
        floor = layout.preamble_symbols != air.long.preamble_symbols  # type: ignore[union-attr]
        for channel in args.channels.split(","):
            for snr in RANGES[channel]:
                acquired = decoded = genie = 0
                for _ in range(args.frames):
                    payload = bytes(rng.integers(0, 256, n, dtype=np.uint8))
                    burst = modem.tx.baseband(header, layout, codec.encode(payload, 0))  # type: ignore[arg-type]
                    buf = np.concatenate((np.zeros(lead, complex), burst, np.zeros(tail, complex)))
                    ch = make_channel(
                        channel,
                        snr_db=float(snr),
                        fs=fs,
                        seed=int(rng.integers(0, 2**31)),
                        signal_power=1.0,
                        cfo_hz=float(rng.uniform(-100, 100)),
                    )
                    y = modem.detector.condition(ch.process(buf))
                    syncs = modem.detector.detect(y, max_frames=1)
                    if (
                        syncs
                        and syncs[0].floor == floor
                        and syncs[0].header.frame_type is header.frame_type
                        and abs(syncs[0].start - lead) <= params.symbol_samples // 2
                    ):
                        acquired += 1
                        try:
                            frame = modem.demodulate(y, syncs[0])
                            out, _ = codec.decode(frame.symbols, frame.noise_var, rv=0)
                            decoded += out == payload
                        except ValueError:
                            pass
                    try:
                        cfo = modem.detector.fine_cfo(
                            y,
                            lead,
                            header.frame_type,
                            layout.preamble_symbols,
                            floor,  # type: ignore[union-attr]
                        )
                        sync = FrameSync(
                            lead, cfo, FrameHeader(header.frame_type), 0.0, 9.0, 1.0, floor
                        )
                        frame = modem.demodulate(y, sync)
                        out, _ = codec.decode(frame.symbols, frame.noise_var, rv=0)
                        genie += out == payload
                    except ValueError:
                        pass
                rows.append(
                    {
                        "frame": label,
                        "payload_bytes": n,
                        "net_bps": round(8 * n / layout.duration_s, 1),  # type: ignore[union-attr]
                        "channel": channel,
                        "snr_3k_db": snr,
                        "frames": args.frames,
                        "acquired": acquired,
                        "decoded": decoded,
                        "genie_decoded": genie,
                    }
                )
                print(
                    f"{label:28s} {channel:8s} {snr:+4d} dB  acq {acquired:2d}/{args.frames}  "
                    f"dec {decoded:2d}  genie {genie:2d}   [{time.time() - t0:5.0f} s]",
                    flush=True,
                )
    out_path = Path(args.out or f"bench/baselines/floor_{args.bandwidth}.csv")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    print("wrote", out_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
