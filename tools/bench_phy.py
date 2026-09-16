"""Frame error rate, throughput and acquisition probability vs SNR (roadmap P1-7).

    python tools/bench_phy.py [--channels awgn,good,moderate,poor] [--modes 0,2,4,6,8,10,13]
                              [--frames 30] [--step 1.0] [--out bench/baselines/phy_fer.csv]
                              [--bandwidth 2300|500]

``--bandwidth 500`` sweeps the narrow air interface (its own ten-mode table, P7-0); the
default output for it is ``bench/baselines/phy_fer_500.csv``. SNR stays referenced to
3 kHz for both, so the two tables are comparable as an operator would compare them: the
same transmitter power into the same noise.

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
from aether_model.frame.modes import AirInterface, air_interface
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WAVEFORMS, Bandwidth

# Rough AWGN thresholds (3 kHz SNR) per mode, used only to pick where a sweep starts.
_START_AWGN = {
    0: -9,
    1: -7,
    2: -5,
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
# The narrow table starts where the wide table's same (modulation, rate) starts, less the
# ≈ 6.8 dB a 500 Hz signal gains per carrier at the same 3 kHz-referenced SNR, and a little.
_START_AWGN_NARROW = {
    0: -16,
    1: -14,
    2: -11,
    3: -9,
    4: -8,
    5: -6,
    6: -4,
    7: -4,
    8: -1,
    9: 0,
    10: 4,
    11: 6,
    12: 7,
}
_FADING_OFFSET = {"awgn": 0.0, "good": 2.0, "moderate": 3.0, "poor": 4.0}


def start_snr(air: AirInterface, mode_idx: int, channel: str) -> float:
    table = _START_AWGN if air.params.bandwidth is Bandwidth.WIDE_2300 else _START_AWGN_NARROW
    return float(table[mode_idx] + _FADING_OFFSET.get(channel, 4.0))


def run_point(
    modem: Modem, channel: str, mode_idx: int, snr_db: float, frames: int, seed0: int
) -> dict[str, float | int | str]:
    mode = modem.modes[mode_idx]
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
            fs=modem.p.fs_baseband,
            seed=seed0 + i,
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-100, 100)),
            sro_ppm=float(rng.uniform(-50, 50)),
        )
        y = ch.process(burst)
        result = modem.decode_buffer(y, max_frames=1)
        if result:
            f = result[0]
            # multipath can lock onto a path up to a few samples late; that still counts
            if abs(f.frame.sync.start - lead) <= 8 and f.frame.mode == mode_idx:
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
        "throughput_bps": round(8 * n_payload * (1 - fer) / modem.air.long.duration_s),
        "mean_reported_snr_db": round(float(np.mean(snr_est)), 2) if snr_est else "",
        "seconds": round(time.perf_counter() - t0, 1),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--modes", default=None, help="mode indices; default: every mode")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--step", type=float, default=1.0)
    ap.add_argument("--max-span", type=float, default=12.0)
    ap.add_argument("--seed", type=int, default=7000)
    ap.add_argument("--bandwidth", type=int, default=2300, choices=(2300, 500))
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    params = WAVEFORMS[Bandwidth(args.bandwidth)]
    modem = Modem(params)
    air = air_interface(params)
    out_default = (
        "bench/baselines/phy_fer.csv"
        if args.bandwidth == 2300
        else ("bench/baselines/phy_fer_500.csv")
    )
    modes_default = (
        "0,2,4,6,8,10,13" if args.bandwidth == 2300 else ",".join(str(m.index) for m in air.modes)
    )
    rows: list[dict[str, float | int | str]] = []
    for channel in args.channels.split(","):
        for mode_s in (args.modes or modes_default).split(","):
            mode_idx = int(mode_s)
            start = start_snr(air, mode_idx, channel)
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
    out = Path(args.out or out_default)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
