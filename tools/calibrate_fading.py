"""Calibrate the fading pipe's effective-SNR mapping to the modem's measured curves (P9-6).

    python tools/calibrate_fading.py [--realizations 400] [--out bench/baselines/fading_pipe.csv]

The fading pipe (``aether_model.link.fading``) puts every frame through its own stretch of
a two-ray ITU-R F.1487 channel and judges it at the exponential effective SNR of its
resource elements. The one free number of that mapping, β, stands for everything the
mapping does not model — the code's use of the diversity, the receiver's channel estimates
on a moving channel — so it is fitted here per frame type and channel class: β is the value
for which the pipe's ensemble frame error rate crosses 10 % exactly where the real modem's
did (``bench/baselines/phy_fer*.csv`` for the data modes through ``bench_link``'s
``channel_thresholds``, ``floor_*.csv`` for the control frames through
``control_thresholds``). A class whose measured point lies outside what any β can reach —
better than a frame's own mean SNR allows, or worse than its worst resource element — is
clamped and flagged.

Columns: bandwidth_hz, frame, channel, awgn_db, target_db, beta, fitted_db, clamped.
"""

from __future__ import annotations

import argparse
import csv
import math
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.fading import STEEP, SharedFading, shapes_for
from aether_model.link.harness import phy_timing
from aether_model.link.phy import Container, TxFrame
from aether_model.link.sim import control_thresholds_for
from bench_link import CONTROL_CSV, channel_thresholds, control_thresholds, table_for

CLASSES = ("good", "moderate", "poor")
BETA_RANGE = (-2.0, 3.0)
"""log10 of the β searched: from far below any modulation's (the worst element decides)
to far above (the mean decides)."""


def ensemble(
    profile: str, duration_s: float, carriers: tuple[float, ...], n: int, seed: int
) -> np.ndarray:
    """``|H|²`` of ``n`` frames at random, well-separated times on one long fade."""
    fading = SharedFading(profile, seed)
    rng = np.random.default_rng(seed + 1)
    starts = np.sort(rng.uniform(0.0, 30.0 * n, n))
    return np.stack([fading.power(t, t + duration_s, carriers) for t in starts])


def fer(snr_db: float, gains: np.ndarray, beta: float, awgn_db: float) -> float:
    """The ensemble's frame error rate at a mean SNR: each frame judged on the AWGN
    waterfall at its effective SNR."""
    x = -(10.0 ** (snr_db / 10.0)) * gains / beta
    top = x.max(axis=(1, 2))
    log_mean = top + np.log(np.mean(np.exp(x - top[:, None, None]), axis=(1, 2)))
    eff = 10.0 * np.log10(np.maximum(-beta * log_mean, 1e-30))
    z = np.clip(STEEP * (eff - awgn_db) + math.log(9.0), -60.0, 60.0)
    return float(np.mean(1.0 - 1.0 / (1.0 + np.exp(-z))))


def crossing(gains: np.ndarray, beta: float, awgn_db: float) -> float:
    """The mean SNR at which the ensemble loses one frame in ten."""
    lo, hi = awgn_db - 10.0, awgn_db + 45.0
    for _ in range(40):
        mid = 0.5 * (lo + hi)
        if fer(mid, gains, beta, awgn_db) > 0.10:
            lo = mid
        else:
            hi = mid
    return 0.5 * (lo + hi)


def calibrate(gains: np.ndarray, awgn_db: float, target_db: float) -> tuple[float, float, bool]:
    """(β, the crossing it gives, clamped): the crossing falls as β grows."""
    lo, hi = BETA_RANGE
    at_lo, at_hi = crossing(gains, 10**lo, awgn_db), crossing(gains, 10**hi, awgn_db)
    if target_db >= at_lo:
        return 10**lo, at_lo, True
    if target_db <= at_hi:
        return 10**hi, at_hi, True
    for _ in range(30):
        mid = 0.5 * (lo + hi)
        if crossing(gains, 10**mid, awgn_db) > target_db:
            lo = mid
        else:
            hi = mid
    beta = 10 ** (0.5 * (lo + hi))
    return beta, crossing(gains, beta, awgn_db), False


def frames_of(air: AirInterface) -> list[tuple[str, TxFrame]]:
    out = [(f"mode {m.index}", TxFrame(Container.DATA, b"", mode=m.index)) for m in air.modes]
    out.append(("control short", TxFrame(Container.CONTROL, b"")))
    if air.floor_short is not None:
        out.append(("control floor", TxFrame(Container.CONTROL, b"", floor=True)))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--realizations", type=int, default=400)
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    rows: list[dict[str, object]] = []
    for air, fer_csv in ((WIDE, "phy_fer.csv"), (NARROW, "phy_fer_500.csv")):
        awgn, _ = table_for(air)
        tables = channel_thresholds(Path("bench/baselines") / fer_csv, awgn=awgn)
        timing = phy_timing(air.params)
        awgn_controls = control_thresholds_for(timing)
        controls = control_thresholds(
            Path(CONTROL_CSV[air]), awgn_controls, tables, awgn, air.floor_long is not None
        )
        shape = shapes_for(air)
        for label, frame in frames_of(air):
            s = shape(frame)
            if frame.container is Container.CONTROL:
                base = awgn_controls[frame.floor]
                targets = {c: controls[c][frame.floor] for c in CLASSES if c in controls}
            else:
                base = awgn[frame.mode]
                targets = {c: tables[c][frame.mode] for c in CLASSES if c in tables}
            for channel, target in targets.items():
                gains = ensemble(channel, s.duration_s, s.carriers_hz, args.realizations, args.seed)
                beta, fitted, clamped = calibrate(gains, base, target)
                rows.append(
                    {
                        "bandwidth_hz": air.params.bandwidth.value,
                        "frame": label,
                        "channel": channel,
                        "awgn_db": round(base, 2),
                        "target_db": round(target, 2),
                        "beta": round(beta, 4),
                        "fitted_db": round(fitted, 2),
                        "clamped": int(clamped),
                    }
                )
                print(
                    f"{air.params.bandwidth.value:5d} Hz {label:14s} {channel:9s} "
                    f"awgn {base:6.1f}  target {target:6.1f}  beta {beta:9.3f}  "
                    f"fit {fitted:6.1f}{'  CLAMPED' if clamped else ''}",
                    flush=True,
                )
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
            w.writeheader()
            w.writerows(rows)
        print(f"\nwrote {out} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
