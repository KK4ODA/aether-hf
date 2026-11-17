"""One line a recording: its live readings' verdicts, drops, and the profile at start and end.

    python -I summary.py INBOX
"""

import glob
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import r3  # noqa: E402


def main():
    for path in sorted(glob.glob(os.path.join(sys.argv[1], "*", "recordings", "*.json"))):
        side = r3.load_sidecar(path)
        bw = side.get("session", {}).get("bandwidth_hz") or 2300
        span = r3.SPANS[bw]
        p = side.get("passband", {})
        readings = p.get("readings", [])
        verdicts = Counter(r[2] for r in readings)
        lows = [r[3] for r in readings if r[3] is not None]
        highs = [r[4] for r in readings if r[4] is not None]
        prof = []
        for name in ("start", "end"):
            pr = r3.profile_from_sidecar(p.get(name))
            if pr is None:
                prof.append(f"{name} -")
                continue
            d = r3.span_drops(pr, span)
            prof.append(f"{name} {d[0]:.1f}/{d[1]:.1f} ({p[name]['looks']})")
        settings = {s["name"]: s["value"] for s in side.get("radio", {}).get("settings", [])}
        dial = side.get("radio", {}).get("dial") or []
        print(
            f"{os.path.basename(path)[:-5]:40s} {side['format'][-2:]} bw{bw} {side['audio']['seconds']:6.1f}s "
            f"{dict(verdicts)} low {min(lows, default=0):.1f}-{max(lows, default=0):.1f} "
            f"high {min(highs, default=0):.1f}-{max(highs, default=0):.1f} | {' | '.join(prof)} "
            f"| dial {dial[0][1] if dial else '-'} {settings}"
        )


main()
