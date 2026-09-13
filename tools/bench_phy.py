"""Frame error rate, throughput and acquisition probability vs SNR (roadmap P1-7).

    python tools/bench_phy.py [--channels awgn,good,moderate,poor] [--modes 0,2,4,6,8,10,13]
                              [--frames 30] [--step 1.0] [--out bench/baselines/phy_fer.csv]

For every (channel, mode) the SNR (3 kHz reference) is swept upward in ``step`` dB from an
estimated starting point until the FER drops below 5 % (or the sweep exceeds 12 dB).
Every frame gets a random carrier offset (±100 Hz), sample-rate offset (±50 ppm) and
timing, so the numbers include acquisition. Fading channels use a fresh seed per frame
(independent fades) but the same seed set for every mode, so mode curves are comparable.

Output rows: channel, mode, snr_3k_db, frames, acquired, decoded, fer, throughput_bps,
mean_reported_snr_db, seconds. "Min usable SNR" tables can be interpolated at FER = 10 %.
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
from aether_model.frame.modes import LONG, MODES
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300

# Rough AWGN thresholds (3 kHz SNR) per mode, used only to pick where a sweep starts.
_START_AWGN = {
    0: -8,
    1: -6,
    2: -4,
    3: -3,
    4: -1,
    5: 1,
    6: 2,
    7: 4,
    8: 4,
    9: 6,
    10: 8,
    11: 11,
    12: 13,
    13: 15,
}
_FADING_OFFSET = {"awgn": 0.0, "good": 2.0, "moderate": 3.0, "poor": 4.0}


def run_point(
    modem: Modem, channel: str, mode_idx: int, snr_db: float, frames: int, seed0: int
) -> dict[str, float | int | str]:
    mode = MODES[mode_idx]
    n_payload = modem.payload_bytes(mode)
    acquired = decoded = 0
    snr_est: list[float] = []
    t0 = time.perf_counter()
    for i in range(frames):
        rng = np.random.default_rng(seed0 + i)
        payload = rng.integers(0, 256, n_payload, dtype=np.uint8).tobytes()
        lead = int(rng.integers(800, 2400))
        burst = np.concatenate((np.zeros(lead), modem.data_burst(payload, mode), np.zeros(1600)))
        ch = make_channel(
            channel,
            snr_db=snr_db,
            fs=WIDE_2300.fs_baseband,
            seed=seed0 + i,
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-100, 100)),
            sro_ppm=float(rng.uniform(-50, 50)),
        )
        y = ch.process(burst)
        result = modem.decode_buffer(y, max_frames=1)
        if result:
            f = result[0]
            if abs(f.frame.sync.start - lead) <= 3 and f.frame.sync.header.mode == mode_idx:
                acquired += 1
                snr_est.append(f.frame.snr_3k_db)
            decoded += int(f.payload == payload)
    fer = 1.0 - decoded / frames
    return {
        "channel": channel,
        "mode": mode_idx,
        "mode_name": mode.name,
        "snr_3k_db": snr_db,
        "frames": frames,
        "acquired": acquired,
        "decoded": decoded,
        "fer": round(fer, 4),
        "throughput_bps": round(8 * n_payload * (1 - fer) / LONG.duration_s),
        "mean_reported_snr_db": round(float(np.mean(snr_est)), 2) if snr_est else "",
        "seconds": round(time.perf_counter() - t0, 1),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--modes", default="0,2,4,6,8,10,13")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--step", type=float, default=1.0)
    ap.add_argument("--max-span", type=float, default=12.0)
    ap.add_argument("--seed", type=int, default=7000)
    ap.add_argument("--out", default="bench/baselines/phy_fer.csv")
    args = ap.parse_args()

    modem = Modem(WIDE_2300)
    rows: list[dict[str, float | int | str]] = []
    for channel in args.channels.split(","):
        for mode_s in args.modes.split(","):
            mode_idx = int(mode_s)
            start = _START_AWGN[mode_idx] + _FADING_OFFSET.get(channel, 4.0)
            snr = float(start)
            good_points = 0
            while snr <= start + args.max_span:
                r = run_point(modem, channel, mode_idx, snr, args.frames, args.seed)
                rows.append(r)
                print(
                    f"{channel:8s} {r['mode_name']:>10} SNR={snr:+5.1f} dB  acq={r['acquired']:2d}/{args.frames}"
                    f"  FER={r['fer']:.3f}  {r['throughput_bps']:5d} bps  "
                    f"(est {r['mean_reported_snr_db']} dB, {r['seconds']} s)",
                    flush=True,
                )
                good_points = good_points + 1 if r["fer"] < 0.05 else 0
                if good_points >= 2:
                    break
                snr += args.step
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
