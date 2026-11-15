# SPDX-License-Identifier: MIT OR Apache-2.0
"""What the manual's screenshot tool (tools/manual/screens.mjs) needs from Python.

    python -m uv run --with pillow python tools/manual/screen_data.py ladder
    python -m uv run --with pillow python tools/manual/screen_data.py shrink PNG...

``ladder`` prints, as JSON, what the daemon's ``capabilities`` method answers for each air: the
rungs with their names, payloads, rates and AWGN thresholds, and the rungs the rate controller
uses. It reads them from the reference model, which the daemon is held to bit for bit, so the
screenshots' mode table and rung names are the program's own and never typed by hand.

``shrink`` quantises screenshots to a palette in place: a panel is flat colour, so 256 colours
keep the text crisp at a fraction of the size, and the manual's PDF carries every picture.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "model"))

from aether_model.frame.modes import NARROW, WIDE, AirInterface  # noqa: E402
from aether_model.link import rate  # noqa: E402


def capabilities(air: AirInterface, thresholds: dict[int, float], hz: int) -> dict[str, object]:
    """The ``capabilities`` answer for one air, as ``core/aetherd/src/control/methods.rs``
    builds it."""
    modes = [
        {
            "index": rung.index,
            "name": rung.name,
            "payload_bytes": rung.payload_bytes,
            "net_bit_rate": rung.net_bps,
            "floor": rung.floor,
            "threshold_db": thresholds[rung.index],
        }
        for rung in air.ladder
    ]
    usable = rate.usable_modes(
        thresholds,
        {rung.index: rung.payload_bytes for rung in air.ladder},
        {rung.index: rung.duration_s for rung in air.ladder},
    )
    return {
        "api_version": "0.1",
        "phy": "aether-hf",
        "bandwidth_hz": hz,
        "bandwidths_hz": [2300, 500],
        "modes": modes,
        "usable_modes": usable,
        "reports_preambles": True,
        "snr_reference_hz": 3000,
    }


def ladder() -> int:
    out = {
        "2300": capabilities(WIDE, dict(rate.AWGN_THRESHOLD_DB), 2300),
        "500": capabilities(NARROW, dict(rate.NARROW_AWGN_THRESHOLD_DB), 500),
    }
    print(json.dumps(out))
    return 0


def shrink(paths: list[str]) -> int:
    # only `shrink` needs Pillow, which uv fetches for it: `ladder` runs without
    from PIL import Image

    for name in paths:
        path = Path(name)
        with Image.open(path) as image:
            flat = image.convert("RGB")
        small = flat.quantize(colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
        small.save(path, optimize=True)
        print(f"{path.name} {path.stat().st_size // 1024} KB")
    return 0


def main() -> int:
    args = sys.argv[1:]
    if args[:1] == ["ladder"]:
        return ladder()
    if args[:1] == ["shrink"]:
        return shrink(args[1:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
