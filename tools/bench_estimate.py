"""ADR-0131's bench: the OFDM receiver's channel estimate with and without the phase slope taken out.

    python -I tools/bench_estimate.py OUT.csv [--arms before,derotate,wiener]
        [--frames 30] [--jobs 4] [--channels awgn,good,moderate,poor,flutter,nvis]
    python -I tools/bench_estimate.py OUT.csv --judge    # read OUT.csv back and judge it

The points are ``bench_noise_shape.py``'s: every air, channel and OFDM mode the committed curves
hold, at the SNRs nearest 30 % and 10 % frame errors, flutter and NVIS at Poor's points. Each
point's frames go through ``bench_phy.run_point`` once per arm, so every arm sees the same frames,
channel and noise. The arms set ``FrameReceiver`` settings in the worker:

``before``    the receiver without de-rotation
``derotate``  the phase slope out before interpolating (ADR-0131 §2.1)
``wiener``    the Wiener interpolator, without de-rotation (E7: is it still worth keeping)

The judgement is ADR-0131 §5's E1–E3 and E7 for de-rotation, decodes summed over a group's points.
The record, ``bench/baselines/channel_estimate.csv``, also holds the arms ``error`` and ``both`` of
§2.2's error term, measured and not built (§8); they are summed and printed, not judged.
"""

from __future__ import annotations

import argparse
import csv
import sys
from collections import defaultdict
from multiprocessing import Pool
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "model"))
sys.path.insert(0, str(ROOT / "tools"))

ARMS: dict[str, tuple[str, ...]] = {
    # the receiver before ADR-0131, named: de-rotation is the default since the port has it
    "before": ("derotate=false",),
    "derotate": ("derotate=true",),
    "wiener": ("channel_estimator=wiener",),
}
ORDER = ("before", "derotate", "error", "both", "wiener")
"""The order arms are printed in, the record's two of the part not built among them."""
CANDIDATE = "derotate"


def run(job: tuple[str, str, str, float, int, int, str]) -> dict[str, object]:
    hz, channel, mode, snr, frames, seed, arm = job
    from aether_model.phy.pipeline import Modem
    from aether_model.waveform import NARROW_500, WIDE_2300
    from bench_cut import configure
    from bench_phy import run_point

    # each worker sets every setting an arm may touch, so an arm never inherits another's
    configure(("derotate=false", "channel_estimator=linear", *ARMS[arm]))
    modem = Modem(WIDE_2300 if hz == "2300" else NARROW_500)
    row = run_point(modem, channel, int(mode), snr, frames, seed)
    return {
        "air": hz,
        "channel": channel,
        "mode": mode,
        "snr_3k_db": snr,
        "arm": arm,
        "frames": frames,
        "decoded": int(row["decoded"]),
        "acquired": int(row["acquired"]),
        "mean_reported_snr_db": row["mean_reported_snr_db"],
    }


def read(path: Path) -> list[dict[str, object]]:
    with path.open(encoding="utf-8") as f:
        return [dict(r) for r in csv.DictReader(f)]


def totals(rows: list[dict[str, object]]) -> dict[tuple[str, str, str], tuple[int, int]]:
    """``(air, channel, arm)`` → ``(decoded, frames)`` summed over the points."""
    out: dict[tuple[str, str, str], list[int]] = defaultdict(lambda: [0, 0])
    for r in rows:
        key = (str(r["air"]), str(r["channel"]), str(r["arm"]))
        out[key][0] += int(str(r["decoded"]))
        out[key][1] += int(str(r["frames"]))
    return {k: (v[0], v[1]) for k, v in out.items()}


def judge(rows: list[dict[str, object]], candidate: str = CANDIDATE) -> bool:
    """ADR-0131 §5's E1–E3 and E7 for ``candidate`` against ``before`` (and ``wiener``)."""
    t = totals(rows)
    airs = sorted({str(r["air"]) for r in rows})
    arms = {str(r["arm"]) for r in rows}
    ok = True

    def dec(air: str, channel: str, arm: str) -> tuple[int, int]:
        return t.get((air, channel, arm), (0, 0))

    def say(name: str, passed: bool, text: str) -> None:
        nonlocal ok
        ok &= passed
        print(f"  {name} {'pass' if passed else 'MISS'}: {text}")

    print(f"\nADR-0131 §5, as registered, for {candidate}:")
    by_point: dict[tuple[str, ...], dict[str, int]] = defaultdict(dict)
    for r in rows:
        key = (str(r["air"]), str(r["channel"]), str(r["mode"]), str(r["snr_3k_db"]))
        by_point[key][str(r["arm"])] = int(str(r["decoded"]))
    # E1: AWGN and Good, no loss beyond 2 % of the frames, and no point more than 3 frames under
    for air in airs:
        for channel in ("awgn", "good"):
            (b, n), (a, _) = dec(air, channel, "before"), dec(air, channel, candidate)
            worst = min(
                (
                    p.get(candidate, 0) - p.get("before", 0)
                    for k, p in by_point.items()
                    if k[0] == air and k[1] == channel
                ),
                default=0,
            )
            say(
                "E1",
                a >= b - 0.02 * n and worst >= -3,
                f"{air} {channel}: {a} vs before {b} of {n}, worst point {worst:+d}",
            )
    # E2: Poor +10 %, Moderate no loss
    for air in airs:
        (b, n), (a, _) = dec(air, "poor", "before"), dec(air, "poor", candidate)
        say("E2", a >= 1.10 * b, f"{air} poor: {a} vs before {b} of {n} ({a / max(b, 1) - 1:+.0%})")
        (b, n), (a, _) = dec(air, "moderate", "before"), dec(air, "moderate", candidate)
        say("E2", a >= b, f"{air} moderate: {a} vs before {b} of {n}")
    # E3: flutter and NVIS, no loss beyond 2 %
    for air in airs:
        for channel in ("flutter", "nvis"):
            (b, n), (a, _) = dec(air, channel, "before"), dec(air, channel, candidate)
            say("E3", a >= b - 0.02 * n, f"{air} {channel}: {a} vs before {b} of {n}")
    # E7: the candidate against the Wiener estimator on every channel, pooled per air
    if "wiener" in arms:
        for air in airs:
            channels = sorted({c for (a_, c, _) in t if a_ == air})
            losing = [c for c in channels if dec(air, c, candidate)[0] < dec(air, c, "wiener")[0]]
            say("E7", not losing, f"{air}: under wiener on {losing or 'no channel'}")
    print(f"\n  {'all pass' if ok else 'a miss: to the author with its trace'}")
    return ok


def summary(rows: list[dict[str, object]]) -> None:
    t = totals(rows)
    present = {str(r["arm"]) for r in rows}
    arms = [a for a in ORDER if a in present] + sorted(present - set(ORDER))
    print("\ndecoded by air and channel, per arm", arms)
    for air, channel in sorted({(a, c) for (a, c, _) in t}):
        cells = "  ".join(f"{arm} {t.get((air, channel, arm), (0, 0))[0]:>4}" for arm in arms)
        n = t.get((air, channel, arms[0]), (0, 0))[1]
        print(f"  {air:>4} {channel:9} of {n:>4}: {cells}")


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):  # Windows' console: the judgement's signs
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--arms", default=",".join(ARMS))
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--channels", default="awgn,good,moderate,poor,flutter,nvis")
    ap.add_argument("--seed", type=int, default=9000)
    ap.add_argument("--judge", action="store_true", help="judge an existing OUT.csv")
    args = ap.parse_args()
    if args.judge:
        rows = read(args.out)
        summary(rows)
        judge(rows)
        return
    arms = args.arms.split(",")
    unknown = set(arms) - set(ARMS)
    if unknown:
        raise SystemExit(f"no such arm: {', '.join(sorted(unknown))} (arms: {', '.join(ARMS)})")
    from bench_noise_shape import ofdm_points

    points = ofdm_points(args.channels.split(","))
    jobs = [
        (hz, channel, mode, snr, args.frames, args.seed + 7919 * i, arm)
        for i, (hz, channel, mode, snr) in enumerate(points)
        for arm in arms
    ]
    rows: list[dict[str, object]] = []
    with args.out.open("w", newline="", encoding="utf-8") as f, Pool(args.jobs) as pool:
        w = None
        for row in pool.imap_unordered(run, jobs):
            if w is None:
                w = csv.DictWriter(f, fieldnames=list(row))
                w.writeheader()
            w.writerow(row)
            f.flush()
            rows.append(row)
            print(
                f"{row['air']:>5} {row['channel']:9} mode {row['mode']:>2} "
                f"{float(str(row['snr_3k_db'])):+6.1f} dB {row['arm']:9} "
                f"{row['decoded']:>3}/{row['frames']}",
                flush=True,
            )
    summary(rows)
    judge(rows)


if __name__ == "__main__":
    main()
