"""Channel-estimator comparison: linear vs robust Wiener (roadmap P2-6).

    python tools/bench_chanest.py [--frames 24] [--modes 4,8,10]
                                  [--channels good,moderate,poor] [--snr-offsets -1,0,1,2]
                                  [--out bench/baselines/chanest.csv]

P2-6 is conditional — "improved channel estimation *if benchmarks justify*" — so this exists
to answer that, not to assume it. It runs the same frames through both estimators:

* ``linear``  — Phase 1: linear interpolation across frequency, 3-tap average in time;
* ``wiener``  — P2-6: robust separable MMSE (`phy/wiener.py`), designed for a worst-case
  6 ms delay spread and 1.5 Hz Doppler rather than for the channel actually present.

Interpolation error only matters where the channel is not flat between pilots, so the
fading channels are the interesting ones; AWGN is included as a control, where the two
should differ only by their noise averaging.

SNR points are given as offsets from each mode's measured AWGN threshold, so every mode is
exercised in the region where frames are actually being won and lost.

Columns: channel, mode, mode_name, snr_db, estimator, frames, decoded, fer, mean_snr_db.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import MODES
from aether_model.link.rate import AWGN_THRESHOLD_DB
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300 as P

FADING_OFFSET = {"awgn": 0.0, "good": 7.0, "moderate": 7.0, "poor": 5.0}
"""Rough penalty of each channel over AWGN, so the swept window lands near the threshold."""


def run_point(
    estimator: str, channel: str, mode_idx: int, snr_db: float, frames: int
) -> tuple[float, float]:
    modem = Modem(P)
    modem.rx.channel_estimator = estimator
    rng = np.random.default_rng(2000 + mode_idx)
    ok = 0
    snrs = []
    for t in range(frames):
        n = modem.payload_bytes(MODES[mode_idx])
        payload = rng.integers(0, 256, n, dtype=np.uint8).tobytes()
        burst = modem.data_burst(payload, MODES[mode_idx])
        lead = int(rng.integers(800, 2400))
        y = make_channel(
            channel,
            snr_db=snr_db,
            fs=P.fs_baseband,
            seed=4000 + t,
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-100, 100)),
            sro_ppm=float(rng.uniform(-50, 50)),
        ).process(np.concatenate((np.zeros(lead), burst, np.zeros(1600))))
        result = modem.decode_buffer(y, max_frames=1)
        if result:
            snrs.append(result[0].frame.snr_3k_db)
            ok += int(result[0].payload == payload)
    return 1.0 - ok / frames, float(np.mean(snrs)) if snrs else float("nan")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--frames", type=int, default=24)
    ap.add_argument("--modes", default="4,8,10")
    ap.add_argument("--channels", default="good,moderate,poor")
    ap.add_argument("--snr-offsets", default="-1,0,1,2")
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    modes = [int(m) for m in args.modes.split(",")]
    channels = [c.strip() for c in args.channels.split(",")]
    offsets = [float(o) for o in args.snr_offsets.split(",")]
    rows: list[dict[str, object]] = []
    wins = {"linear": 0.0, "wiener": 0.0}

    for channel in channels:
        print(f"\n=== {channel} ===")
        print("  mode          SNR    linear   wiener    delta")
        for mode_idx in modes:
            base = AWGN_THRESHOLD_DB[mode_idx] + FADING_OFFSET.get(channel, 5.0)
            for off in offsets:
                snr = base + off
                fers = {}
                for estimator in ("linear", "wiener"):
                    fer, mean_snr = run_point(estimator, channel, mode_idx, snr, args.frames)
                    fers[estimator] = fer
                    rows.append(
                        {
                            "channel": channel,
                            "mode": mode_idx,
                            "mode_name": MODES[mode_idx].name,
                            "snr_db": round(snr, 1),
                            "estimator": estimator,
                            "frames": args.frames,
                            "decoded": round((1 - fer) * args.frames),
                            "fer": round(fer, 4),
                            "mean_snr_db": round(mean_snr, 2) if mean_snr == mean_snr else "",
                        }
                    )
                delta = fers["linear"] - fers["wiener"]  # positive = Wiener decoded more
                wins["wiener" if delta > 0 else "linear"] += abs(delta)
                print(
                    f"  {MODES[mode_idx].name:<10} {snr:+6.1f}   {fers['linear']:6.2f}   "
                    f"{fers['wiener']:6.2f}   {delta:+6.2f}",
                    flush=True,
                )

    total = wins["wiener"] - wins["linear"]
    print(f"\nnet FER improvement from Wiener, summed over points: {total:+.2f}")
    print("Wiener wins overall" if total > 0 else "linear is as good or better; keep it")

    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
        print(f"wrote {out} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
