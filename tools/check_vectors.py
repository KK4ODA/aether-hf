"""Check that the committed cross-validation vectors still describe the model.

Regenerates every vector file into a temporary directory and compares it with the committed
one. What this is guarding against is a change to the model that was never propagated to the
Rust core: the vectors are how the two implementations are held together, and a stale vector
file silently stops testing anything.

Byte equality cannot be the test. The generators go through transcendental functions, and a C
library's last bit differs between platforms — the committed preamble tables were produced on
Windows, and Linux computes a handful of Zadoff-Chu values one unit in the last place away.
Comparing the files byte for byte therefore fails on a difference that means nothing, which
trains everybody to ignore the check.

So the comparison is structural: every integer, string, boolean and key identical, and every
float within a tolerance far tighter than any test that reads these files. A real model change
moves a value by orders of magnitude more than this and is still caught.
"""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

TOLERANCE = 1e-12
"""Relative tolerance on floats: a few units in the last place.

Three orders of magnitude tighter than the 1e-9 the Rust tests compare at, so anything this
accepts is invisible to everything downstream, and anything a model change would cause is
far larger."""

GENERATORS: list[tuple[str, str]] = [
    ("tools/make_fec_vectors.py", "core/aether-fec/tests/data/fec_vectors.json"),
    ("tools/make_phy_vectors.py", "core/aether-phy/tests/data/phy_vectors.json"),
    ("tools/make_phy_tables.py", "core/aether-phy/data/preamble_tables.json"),
    ("tools/make_link_vectors.py", "core/aether-link/tests/data/link_vectors.json"),
]


def close(a: float, b: float) -> bool:
    return math.isclose(a, b, rel_tol=TOLERANCE, abs_tol=TOLERANCE)


def compare(committed: object, fresh: object, path: str, out: list[str]) -> None:
    """Walk two decoded JSON documents, recording every difference that matters."""
    if isinstance(committed, bool) or isinstance(fresh, bool):
        # bool before int: in Python a bool *is* an int, and True == 1 would slip through
        if committed is not fresh:
            out.append(f"{path}: {committed!r} vs {fresh!r}")
        return
    if isinstance(committed, (int, float)) and isinstance(fresh, (int, float)):
        if isinstance(committed, int) and isinstance(fresh, int):
            if committed != fresh:
                out.append(f"{path}: {committed} vs {fresh}")
        elif not close(float(committed), float(fresh)):
            out.append(f"{path}: {committed!r} vs {fresh!r}")
        return
    if isinstance(committed, dict) and isinstance(fresh, dict):
        for key in sorted(set(committed) | set(fresh)):
            if key not in committed:
                out.append(f"{path}.{key}: missing from the committed file")
            elif key not in fresh:
                out.append(f"{path}.{key}: no longer generated")
            else:
                compare(committed[key], fresh[key], f"{path}.{key}", out)
        return
    if isinstance(committed, list) and isinstance(fresh, list):
        if len(committed) != len(fresh):
            out.append(f"{path}: {len(committed)} entries vs {len(fresh)}")
            return
        for index, (a, b) in enumerate(zip(committed, fresh, strict=True)):
            compare(a, b, f"{path}[{index}]", out)
        return
    if committed != fresh:
        out.append(f"{path}: {committed!r} vs {fresh!r}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument(
        "--max-report",
        type=int,
        default=20,
        help="how many differences to print before stopping",
    )
    args = ap.parse_args()

    root = Path(__file__).resolve().parent.parent
    failures = 0
    with tempfile.TemporaryDirectory() as tmp:
        for generator, committed_path in GENERATORS:
            fresh_path = Path(tmp) / Path(committed_path).name
            result = subprocess.run(
                [sys.executable, str(root / generator), "--out", str(fresh_path)],
                cwd=root,
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode != 0:
                print(f"{generator}: failed to run\n{result.stderr}", file=sys.stderr)
                failures += 1
                continue

            committed = json.loads((root / committed_path).read_text(encoding="utf-8"))
            fresh = json.loads(fresh_path.read_text(encoding="utf-8"))
            differences: list[str] = []
            compare(committed, fresh, Path(committed_path).name, differences)
            if differences:
                failures += 1
                print(f"{committed_path} no longer matches the model:", file=sys.stderr)
                for line in differences[: args.max_report]:
                    print(f"  {line}", file=sys.stderr)
                if len(differences) > args.max_report:
                    print(
                        f"  ... and {len(differences) - args.max_report} more",
                        file=sys.stderr,
                    )
            else:
                print(f"{committed_path}: current")

    if failures:
        print(
            f"\n{failures} vector file(s) are stale. Regenerate them with the tools in "
            "tools/, and only for a deliberate model change (ADR-0001).",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
