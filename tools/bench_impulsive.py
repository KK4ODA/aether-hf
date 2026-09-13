"""Impulsive-noise defence benchmark (roadmap P2-5).

    python tools/bench_impulsive.py [--frames 12] [--modes 0,4,10]
                                    [--probs 0,0.002,0.005,0.01,0.02,0.05,0.1,0.2]
                                    [--burst-db 25] [--out bench/baselines/impulsive.csv]

HF is full of impulsive noise, and it is the impairment OFDM handles *worse* than a
single-carrier waveform: the FFT spreads one hot sample across all 57 carriers of the symbol
it lands in. Two defences were added in P2-5 and this measures them separately, because they
do very different amounts of work:

* **blanker** — a median-referenced time-domain blanker ahead of the band-limiting filter
  (`phy/blanker.py`);
* **per-symbol noise variance** — the receiver estimates σ² per OFDM symbol instead of once
  per frame, so a damaged symbol's LLRs shrink and it becomes an erasure (`phy/rx.py`).

The interesting control is the clean channel: a defence that costs sensitivity when there is
nothing to defend against is not one you can leave switched on.

Columns: mode, mode_name, snr_db, impulsive_probability, burst_db, blanker, per_symbol_noise,
frames, decoded, fer, blanked_fraction.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import ChannelConfig, HfChannel
from aether_model.frame.modes import MODES
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300 as P

# SNR at which each mode is comfortably above its threshold, so what the sweep measures is
# the impulsive noise and not the thermal margin.
OPERATING_SNR = {0: -3.0, 2: 0.0, 4: 4.0, 6: 7.0, 8: 9.0, 10: 13.0, 13: 20.0}


def run_point(
    mode_idx: int,
    snr_db: float,
    probability: float,
    burst_db: float,
    blanker: bool,
    per_symbol: bool,
    frames: int,
) -> tuple[float, float]:
    modem = Modem(P, blank_impulses=blanker)
    if not per_symbol:
        modem.rx.noise_shrinkage = 1.0  # one variance for the whole frame (pre-P2-5)
    rng = np.random.default_rng(50 + mode_idx)
    ok = 0
    blanked = []
    for t in range(frames):
        n = modem.payload_bytes(MODES[mode_idx])
        payload = rng.integers(0, 256, n, dtype=np.uint8).tobytes()
        burst = modem.data_burst(payload, MODES[mode_idx])
        cfg = ChannelConfig(
            profile="awgn",
            snr_db=snr_db,
            fs=P.fs_baseband,
            seed=900 + t,
            signal_power=1.0,
            impulsive_probability=probability,
            impulsive_db_above_noise=burst_db,
        )
        y = HfChannel(cfg).process(np.concatenate((np.zeros(900), burst, np.zeros(900))))
        if modem.blanker is not None:
            blanked.append(modem.blanker.process(y).fraction)
        result = modem.decode_buffer(y)
        ok += int(len(result) == 1 and result[0].payload == payload)
    return 1.0 - ok / frames, float(np.mean(blanked)) if blanked else 0.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--frames", type=int, default=12)
    ap.add_argument("--modes", default="0,4,10")
    ap.add_argument("--probs", default="0,0.002,0.005,0.01,0.02,0.05,0.1,0.2")
    ap.add_argument("--burst-db", type=float, default=25.0)
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    modes = [int(m) for m in args.modes.split(",")]
    probs = [float(p) for p in args.probs.split(",")]
    rows: list[dict[str, object]] = []

    for mode_idx in modes:
        snr = OPERATING_SNR.get(mode_idx, 10.0)
        print(
            f"\nmode {mode_idx} ({MODES[mode_idx].name}) at {snr:+.1f} dB, bursts "
            f"{args.burst_db:.0f} dB above noise"
        )
        print("  prob      none    per-sym  blanker   both    blanked%")
        for prob in probs:
            cells = []
            frac = 0.0
            for blanker, per_symbol in ((False, False), (False, True), (True, False), (True, True)):
                fer, f = run_point(
                    mode_idx, snr, prob, args.burst_db, blanker, per_symbol, args.frames
                )
                cells.append(fer)
                frac = max(frac, f)
                rows.append(
                    {
                        "mode": mode_idx,
                        "mode_name": MODES[mode_idx].name,
                        "snr_db": snr,
                        "impulsive_probability": prob,
                        "burst_db": args.burst_db,
                        "blanker": int(blanker),
                        "per_symbol_noise": int(per_symbol),
                        "frames": args.frames,
                        "decoded": round((1 - fer) * args.frames),
                        "fer": round(fer, 4),
                        "blanked_fraction": round(f, 5),
                    }
                )
            print(
                f"  {prob:<8.4f}  "
                + "  ".join(f"{c:6.2f} " for c in cells)
                + f"  {frac * 100:5.2f}",
                flush=True,
            )

    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {out} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
