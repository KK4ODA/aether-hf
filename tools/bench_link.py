"""Link-layer throughput and rate-control benchmark (roadmap P2-2).

    python tools/bench_link.py [--backend sim|phy] [--channels awgn,good,moderate,poor]
                               [--snr -4,0,4,8,12,16,20] [--bytes 8000] [--trials 3]
                               [--ramp] [--out bench/baselines/link_throughput.csv]

For every (channel, SNR) it runs a complete session — connect, transfer ``bytes``, orderly
disconnect — and reports the goodput the *link layer* actually achieved, which is the number
a user sees: PHY air time plus ACK turnarounds, retransmissions, HARQ combining and whatever
the rate controller chose along the way.

Two backends:

* ``sim`` (default) — the DSP-free lossy pipe, calibrated per channel from the measured PHY
  sweep (``bench/baselines/phy_fer.csv``): a whole grid in seconds, good for rate-control
  behaviour and regressions in the protocol itself.
* ``phy`` — the real modem and channel simulator, frame by frame. Minutes per point, but the
  numbers are end-to-end truth.

``--ramp`` replaces the fixed-SNR runs with a triangular fade (``--ramp-span`` dB peak to
trough over ``--ramp-period`` s) and reports how the controller tracked it: the modes it
used, how often it changed them, and what that cost in retransmissions.

Output columns: backend, channel, snr_db (or ramp description), bytes, seconds, goodput_bps,
ideal_bps, efficiency, mode_min/mode_max/mode_final, mode_changes, bursts, frames_sent,
frames_resent, harq_rescues, ack_timeouts, ok.
"""

from __future__ import annotations

import argparse
import csv
import statistics
import sys
import time
from collections import defaultdict
from itertools import pairwise
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import LONG, MODES
from aether_model.link.engine import LinkEngine
from aether_model.link.harness import phy_timing, two_modem_sim
from aether_model.link.rate import AWGN_THRESHOLD_DB, usable_modes
from aether_model.link.sim import TwoStationSim

CHANNEL_ORDER = ("awgn", "good", "moderate", "poor")


# ── channel calibration for the fast backend ──────────────────────────


def channel_thresholds(csv_path: Path, target_fer: float = 0.10) -> dict[str, dict[int, float]]:
    """Per-channel, per-mode minimum usable SNR interpolated from the PHY sweep.

    Modes the sweep did not cover are filled by shifting the AWGN table by the mean measured
    penalty of that channel — crude, but it keeps the whole mode ladder available to the rate
    controller instead of leaving holes it would have to skip.
    """
    if not csv_path.exists():
        return {}
    with csv_path.open(encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    points: dict[tuple[str, int], list[tuple[float, float]]] = defaultdict(list)
    for r in rows:
        points[(r["channel"], int(r["mode"]))].append((float(r["snr_3k_db"]), float(r["fer"])))

    def crossing(pts: list[tuple[float, float]]) -> float | None:
        prev: tuple[float, float] | None = None
        for snr, fer in sorted(pts):
            if fer <= target_fer:
                if prev is None or prev[1] == fer:
                    return snr
                s0, f0 = prev
                return s0 + (f0 - target_fer) / (f0 - fer) * (snr - s0)
            prev = (snr, fer)
        return None

    measured: dict[str, dict[int, float]] = defaultdict(dict)
    for (channel, mode), pts in points.items():
        x = crossing(pts)
        if x is not None:
            measured[channel][mode] = x

    out: dict[str, dict[int, float]] = {}
    for channel, table in measured.items():
        penalties = [table[m] - AWGN_THRESHOLD_DB[m] for m in table if m in AWGN_THRESHOLD_DB]
        shift = statistics.fmean(penalties) if penalties else 0.0
        out[channel] = {m: table.get(m, AWGN_THRESHOLD_DB[m] + shift) for m in AWGN_THRESHOLD_DB}
    return out


def ideal_bps(thresholds: dict[int, float], snr_db: float) -> float:
    """Payload rate of the fastest mode the channel supports at this SNR, ignoring every
    protocol cost — the ceiling the link layer is measured against."""
    best = 0.0
    for m in usable_modes():
        if thresholds.get(m, AWGN_THRESHOLD_DB[m]) <= snr_db:
            best = max(best, MODES[m].net_bit_rate(LONG))
    return best


# ── one run ───────────────────────────────────────────────────────────


def run_point(
    backend: str,
    channel: str,
    snr_db: float,
    payload: bytes,
    seed: int,
    thresholds: dict[int, float] | None,
    ramp: tuple[float, float] | None = None,
) -> dict[str, object]:
    timing = phy_timing()
    a = LinkEngine("W4ODA", timing, seed=seed)
    b = LinkEngine("KK4XYZ", timing, seed=seed + 1)
    schedule = None
    if ramp is not None:
        span, period = ramp
        top, half = snr_db + span / 2, period / 2

        def schedule(t: float, top: float = top, span: float = span, half: float = half) -> float:
            phase = t % (2 * half)
            return top - span * phase / half if phase < half else top - span * (2 - phase / half)

    if backend == "phy":
        sim = two_modem_sim(a, b, channel=channel, snr_db=snr_db, seed=seed)
        sim.snr_schedule = schedule
    else:
        sim = TwoStationSim(
            a, b, snr_db=snr_db, seed=seed, thresholds=thresholds, snr_schedule=schedule
        )

    modes: list[int] = []
    original = a._send_burst

    def wrapped() -> None:
        modes.append(min(a._recommended, a.cfg.max_mode))
        original()

    a._send_burst = wrapped  # type: ignore[method-assign]

    wall = time.perf_counter()
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    seconds = sim.run(until=80.0 * len(payload) / 1000.0 + 400.0)
    ok = sim.delivered(1) == payload
    changes = sum(1 for x, y in pairwise(modes) if x != y)
    goodput = 8 * len(sim.delivered(1)) / seconds if seconds > 0 else 0.0
    ceiling = ideal_bps(thresholds or AWGN_THRESHOLD_DB, snr_db)
    return {
        "backend": backend,
        "channel": channel,
        "snr_db": round(snr_db, 1),
        "ramp": "" if ramp is None else f"+/-{ramp[0] / 2:.0f}dB/{ramp[1]:.0f}s",
        "bytes": len(payload),
        "seconds": round(seconds, 1),
        "goodput_bps": round(goodput, 1),
        "ideal_bps": round(ceiling, 1),
        "efficiency": round(goodput / ceiling, 3) if ceiling else "",
        "mode_min": min(modes) if modes else "",
        "mode_max": max(modes) if modes else "",
        "mode_final": modes[-1] if modes else "",
        "mode_changes": changes,
        "bursts": a.stats.bursts,
        "frames_sent": a.stats.frames_sent,
        "frames_resent": a.stats.frames_resent,
        "harq_rescues": b.stats.harq_rescues,
        "ack_timeouts": a.stats.ack_timeouts,
        "ok": int(ok),
        "wall_s": round(time.perf_counter() - wall, 1),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--backend", choices=("sim", "phy"), default="sim")
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--snr", default="-4,0,4,8,12,16,20")
    ap.add_argument("--bytes", type=int, default=8000)
    ap.add_argument("--trials", type=int, default=3)
    ap.add_argument("--ramp", action="store_true", help="triangular fade instead of a fixed SNR")
    ap.add_argument("--ramp-span", type=float, default=16.0)
    ap.add_argument("--ramp-period", type=float, default=60.0)
    ap.add_argument("--fer-csv", default="bench/baselines/phy_fer.csv")
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    tables = channel_thresholds(Path(args.fer_csv))
    if args.backend == "sim" and not tables:
        print(f"warning: {args.fer_csv} not found; every channel modelled as AWGN", flush=True)
    channels = [c.strip() for c in args.channels.split(",") if c.strip()]
    snrs = [float(s) for s in args.snr.split(",") if s.strip()]
    payload = bytes((i * 37) % 256 for i in range(args.bytes))
    ramp = (args.ramp_span, args.ramp_period) if args.ramp else None

    rows: list[dict[str, object]] = []
    for channel in sorted(
        channels, key=lambda c: CHANNEL_ORDER.index(c) if c in CHANNEL_ORDER else 9
    ):
        thresholds = tables.get(channel)
        for snr in snrs:
            for trial in range(args.trials):
                row = run_point(
                    args.backend, channel, snr, payload, 100 + 7 * trial, thresholds, ramp
                )
                rows.append(row)
                print(
                    f"{channel:9s} SNR {snr:+5.1f} dB  {row['goodput_bps']:7.1f} bps "
                    f"(eff {row['efficiency']})  modes {row['mode_min']}-{row['mode_max']} "
                    f"({row['mode_changes']} changes)  resent {row['frames_resent']}  "
                    f"ok={row['ok']}  [{row['wall_s']} s wall]",
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
