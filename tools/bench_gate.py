"""The benchmark regression gate: no curve may get worse by more than 0.3 dB.

    python tools/bench_gate.py [--baseline bench/baselines/gate_awgn.csv] [--csv RESULTS]
                               [--tolerance 0.3] [--regenerate]

A release is gated on the modem not having got worse. This runs a fixed, reduced grid of
`tools/bench_phy.py` — AWGN, four modes spanning the table, thirty frames a point — finds
the SNR at which each mode's frame error rate crosses 10 %, and compares it with the
committed baseline measured the same way. The sweep is seeded, so the same code gives the
same numbers: a difference is a change in the modem, not in the dice. The grid is deliberately
small (about a quarter of an hour on a runner) because it runs on every release; the full
grid in `bench/README.md` is the place to look when this one says something moved.

`--csv` compares an already-completed run instead of sweeping; `--regenerate` writes the
baseline from a fresh sweep, which is the right thing to do only after a deliberate change
to the waveform — with the reason in the commit that does it.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from update_rate_table import thresholds

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "bench" / "baselines" / "gate_awgn.csv"
GRID = ["--channels", "awgn", "--modes", "0,4,8,13", "--frames", "30"]
TARGET_FER = 0.10


def sweep(out: Path) -> None:
    subprocess.run(
        [sys.executable, str(ROOT / "tools" / "bench_phy.py"), *GRID, "--out", str(out)],
        check=True,
        cwd=ROOT,
    )


def compare(baseline: Path, fresh: Path, tolerance: float) -> int:
    before, _ = thresholds(baseline, TARGET_FER)
    after, after_unresolved = thresholds(fresh, TARGET_FER)
    worst = 0.0
    failed = False
    print(f"{'mode':>4}  {'baseline':>9}  {'now':>7}  {'change':>7}")
    # the modes the sweep measured; a baseline may know more modes than the gate's grid
    for mode in sorted(set(after) | after_unresolved):
        if mode in after_unresolved:
            print(f"{mode:>4}  {before.get(mode, float('nan')):>9.2f}  {'never':>7}  {'FAIL':>7}")
            failed = True
            continue
        if mode not in before:
            print(f"{mode:>4}  {'none':>9}  {after[mode]:>7.2f}  {'new':>7}")
            continue
        change = after[mode] - before[mode]
        worst = max(worst, change)
        verdict = "FAIL" if change > tolerance else ""
        failed |= change > tolerance
        print(f"{mode:>4}  {before[mode]:>9.2f}  {after[mode]:>7.2f}  {change:>+7.2f} {verdict}")
    if failed:
        print(f"a mode got worse by more than {tolerance} dB at FER {TARGET_FER:.0%}")
        return 1
    print(f"ok: worst change {worst:+.2f} dB, within {tolerance} dB")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--baseline", type=Path, default=BASELINE)
    parser.add_argument("--csv", type=Path, help="compare this run instead of sweeping")
    parser.add_argument("--tolerance", type=float, default=0.3, help="dB a mode may lose")
    parser.add_argument("--regenerate", action="store_true", help="rewrite the baseline")
    args = parser.parse_args()

    if args.regenerate:
        sweep(args.baseline)
        print(f"baseline written to {args.baseline}")
        return 0
    if args.csv is not None:
        return compare(args.baseline, args.csv, args.tolerance)
    with tempfile.TemporaryDirectory() as tmp:
        fresh = Path(tmp) / "gate.csv"
        sweep(fresh)
        return compare(args.baseline, fresh, args.tolerance)


if __name__ == "__main__":
    sys.exit(main())
