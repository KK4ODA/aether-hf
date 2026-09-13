"""Regenerate the rate controller's AWGN threshold table from a PHY sweep (roadmap P2-2b).

    python tools/update_rate_table.py [--csv bench/baselines/phy_fer_awgn14.csv]
                                      [--fer 0.10] [--apply]

``link/rate.py`` picks modes from a table of minimum usable SNR per mode. Any entry that is
interpolated rather than measured is a guess the rate controller will act on as if it were
fact — and the P2-4 work found two of them (modes 9 and 12) to be optimistic enough to cost
frames. This reads a sweep that covers *every* mode and prints the measured table, so the
guesses can be replaced with numbers.

Without ``--apply`` it only prints; with it, the ``AWGN_THRESHOLD_DB`` literal in
``model/aether_model/link/rate.py`` is rewritten in place.
"""

from __future__ import annotations

import argparse
import csv
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import MODES

ROOT = Path(__file__).resolve().parents[1]
RATE_PY = ROOT / "model" / "aether_model" / "link" / "rate.py"


def thresholds(csv_path: Path, target_fer: float) -> tuple[dict[int, float], set[int]]:
    """Interpolated FER-crossing per mode, plus the set of modes the sweep never got below
    the target for (those keep whatever the table already had)."""
    with csv_path.open(encoding="utf-8") as f:
        rows = [r for r in csv.DictReader(f) if r["channel"] == "awgn"]
    points: dict[int, list[tuple[float, float]]] = defaultdict(list)
    for r in rows:
        points[int(r["mode"])].append((float(r["snr_3k_db"]), float(r["fer"])))
    out: dict[int, float] = {}
    unresolved: set[int] = set()
    for mode, pts in points.items():
        prev: tuple[float, float] | None = None
        for snr, fer in sorted(pts):
            if fer <= target_fer:
                if prev is None or prev[1] == fer:
                    out[mode] = snr
                else:
                    s0, f0 = prev
                    out[mode] = s0 + (f0 - target_fer) / (f0 - fer) * (snr - s0)
                break
            prev = (snr, fer)
        else:
            unresolved.add(mode)
    return out, unresolved


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--csv", default="bench/baselines/phy_fer_awgn14.csv")
    ap.add_argument("--fer", type=float, default=0.10)
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    measured, unresolved = thresholds(Path(args.csv), args.fer)
    current = re.search(
        r"AWGN_THRESHOLD_DB: dict\[int, float\] = \{(.*?)\n\}",
        RATE_PY.read_text(encoding="utf-8"),
        re.S,
    )
    old = (
        {int(k): float(v) for k, v in re.findall(r"(\d+):\s*(-?[\d.]+)", current.group(1))}
        if current
        else {}
    )

    print(f"{'mode':>4} {'name':<12} {'old':>7} {'measured':>9} {'shift':>7}")
    lines = []
    for mode in MODES:
        i = mode.index
        if i in measured:
            value = round(measured[i], 1)
            shift = f"{value - old.get(i, value):+.1f}"
        else:
            value = old.get(i, 0.0)
            shift = "(kept)" if i in unresolved else "(absent)"
        lines.append(f"    {i}: {value},")
        print(f"{i:>4} {mode.name:<12} {old.get(i, float('nan')):>7.1f} {value:>9.1f} {shift:>7}")
    if unresolved:
        print(f"\nnever reached FER <= {args.fer:.2f} in the sweep: {sorted(unresolved)}")

    block = "AWGN_THRESHOLD_DB: dict[int, float] = {\n" + "\n".join(lines) + "\n}"
    if args.apply:
        text = RATE_PY.read_text(encoding="utf-8")
        text = re.sub(
            r"AWGN_THRESHOLD_DB: dict\[int, float\] = \{.*?\n\}", block, text, count=1, flags=re.S
        )
        RATE_PY.write_text(text, encoding="utf-8", newline="\n")
        print(f"\nrewrote {RATE_PY.relative_to(ROOT)}")
    else:
        print("\n" + block + "\n\n(run with --apply to write it)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
