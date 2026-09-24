"""Regenerate the rate controller's AWGN threshold table from the PHY sweeps (roadmap P2-2b).

    python tools/update_rate_table.py [--csv bench/baselines/phy_fer_awgn14.csv]
                                      [--tone-csv bench/baselines/tone_floor.csv]
                                      [--fer 0.10] [--apply] [--bandwidth 2300|500]

``link/rate.py`` picks rungs of the ladder from a table of minimum usable SNR per rung. Any
entry that is interpolated rather than measured is a guess the rate controller will act on
as if it were fact — and the P2-4 work found two of them to be optimistic enough to cost
frames. This reads a sweep that covers *every* OFDM mode (``bench_phy.py``, which names OFDM
modes) and the tone floor's (``bench_tone.py``, ADR-0013), maps both onto the air's ladder,
and prints the measured table, so the guesses can be replaced with numbers.

Without ``--apply`` it only prints; with it, the ``AWGN_THRESHOLD_DB`` literal (or the
narrow one) and ``TONE_CONTROL_THRESHOLD_DB`` in ``model/aether_model/link/rate.py`` are
rewritten in place.
"""

from __future__ import annotations

import argparse
import csv
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import NARROW, TONE_CONTROL, WIDE

ROOT = Path(__file__).resolve().parents[1]
RATE_PY = ROOT / "model" / "aether_model" / "link" / "rate.py"


def thresholds(csv_path: Path, target_fer: float) -> tuple[dict[int, float], set[int]]:
    """Interpolated FER-crossing per OFDM mode, plus the set of modes the sweep never got
    below the target for (those keep whatever the table already had)."""
    with csv_path.open(encoding="utf-8") as f:
        rows = [r for r in csv.DictReader(f) if r["channel"] == "awgn"]
    points: dict[int, list[tuple[float, float]]] = defaultdict(list)
    for r in rows:
        points[int(r["mode"])].append((float(r["snr_3k_db"]), float(r["fer"])))
    return crossings(points, target_fer)


def tone_thresholds(csv_path: Path, target_fer: float) -> tuple[dict[str, float], set[str]]:
    """The same for the tone floor's kinds, by name, from frames decoded through the
    detector."""
    points: dict[str, list[tuple[float, float]]] = defaultdict(list)
    with csv_path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            if r["channel"] == "awgn":
                fer = 1.0 - int(r["decoded"]) / max(int(r["frames"]), 1)
                points[r["frame"]].append((float(r["snr_3k_db"]), fer))
    return crossings(points, target_fer)


def crossings[K](
    points: dict[K, list[tuple[float, float]]], target_fer: float
) -> tuple[dict[K, float], set[K]]:
    out: dict[K, float] = {}
    unresolved: set[K] = set()
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
    ap.add_argument("--csv", default=None)
    ap.add_argument("--tone-csv", default="bench/baselines/tone_floor.csv")
    ap.add_argument("--fer", type=float, default=0.10)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--bandwidth", type=int, default=2300, choices=(2300, 500))
    args = ap.parse_args()

    # the narrow ladder is its own literal, with its own sweep and its own OFDM modes; the
    # tone floor's rungs are the same frames on both
    narrow = args.bandwidth == 500
    name = "NARROW_AWGN_THRESHOLD_DB" if narrow else "AWGN_THRESHOLD_DB"
    air = NARROW if narrow else WIDE
    csv_path = Path(
        args.csv
        or ("bench/baselines/phy_fer_500.csv" if narrow else "bench/baselines/phy_fer_awgn14.csv")
    )
    ofdm, ofdm_unresolved = thresholds(csv_path, args.fer)
    tones, tone_unresolved = tone_thresholds(Path(args.tone_csv), args.fer)
    measured: dict[int, float] = {}
    unresolved: set[int] = set()
    for r in air.ladder:
        key = r.tone.name if r.tone is not None else r.mode.index  # type: ignore[union-attr]
        found = tones if r.tone is not None else ofdm
        if key in found:
            measured[r.index] = found[key]  # type: ignore[index]
        elif key in (tone_unresolved if r.tone is not None else ofdm_unresolved):
            unresolved.add(r.index)
    current = re.search(
        rf"\n{name}: dict\[int, float\] = \{{(.*?)\n\}}",
        RATE_PY.read_text(encoding="utf-8"),
        re.S,
    )
    old = (
        {int(k): float(v) for k, v in re.findall(r"(\d+):\s*(-?[\d.]+)", current.group(1))}
        if current
        else {}
    )

    print(f"{'rung':>4} {'name':<12} {'old':>7} {'measured':>9} {'shift':>7}")
    lines = []
    for mode in air.ladder:
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

    block = f"{name}: dict[int, float] = {{\n" + "\n".join(lines) + "\n}"
    control = tones.get(TONE_CONTROL.name)
    if control is not None:
        print(f"\n{TONE_CONTROL.name}: {control:.1f} dB")
    if args.apply:
        text = RATE_PY.read_text(encoding="utf-8")
        text = re.sub(
            rf"\n{name}: dict\[int, float\] = \{{.*?\n\}}",
            "\n" + block,
            text,
            count=1,
            flags=re.S,
        )
        if control is not None:
            text = re.sub(
                r"\nTONE_CONTROL_THRESHOLD_DB = -?[\d.]+",
                f"\nTONE_CONTROL_THRESHOLD_DB = {round(control, 1)}",
                text,
                count=1,
            )
        RATE_PY.write_text(text, encoding="utf-8", newline="\n")
        print(f"\nrewrote {RATE_PY.relative_to(ROOT)}")
    else:
        print("\n" + block + "\n\n(run with --apply to write it)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
