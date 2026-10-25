"""A rolling channel's manifest only moves forward.

    python tools/channel_order.py dist/latest.json channel-now/latest.json

The beta channel is one release (`channel-beta`) whose `latest.json` every beta installation
reads. Each `-beta.N` tag's run points it at its own manifest, and two tags pushed together
build side by side: whichever run finishes last would win, and an older beta finishing
second would tell every installation that it is the newest — beta.123's and beta.124's
runs started in the same second on 2026-10-25. The release step asks this first.

Exits 0 when the first manifest's version is at least the second's (a re-run of the same
tag still repairs the channel), or when the second cannot be read (a new channel); 1 when
the first is older, saying which is there.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?$")
# a pre-release sorts below its version, and alpha < beta < rc (semver's identifiers)
STAGES = {"alpha": 0, "beta": 1, "rc": 2}


def order(version: str) -> tuple[int, int, int, int, int]:
    """Semver precedence for the versions `tools/release.py` cuts; a stable release is above
    every pre-release of it."""
    found = VERSION.match(version.strip().lstrip("v"))
    if found is None:
        raise ValueError(f"not a release version: {version!r}")
    major, minor, patch, stage, number = found.groups()
    if stage is None:
        return (int(major), int(minor), int(patch), len(STAGES), 0)
    return (int(major), int(minor), int(patch), STAGES[stage], int(number))


def version_of(path: Path) -> str | None:
    try:
        version = json.loads(path.read_text(encoding="utf-8")).get("version")
    except (OSError, ValueError, AttributeError):
        return None
    return version if isinstance(version, str) else None


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    ours = version_of(Path(sys.argv[1]))
    if ours is None:
        print(f"::error::{sys.argv[1]} states no version")
        return 2
    there = version_of(Path(sys.argv[2]))
    if there is None:
        print(f"the channel states no version yet; {ours} goes on it")
        return 0
    try:
        older = order(ours) < order(there)
    except ValueError as error:
        print(f"::error::{error}")
        return 2
    if older:
        print(f"the channel already offers {there}, newer than {ours}")
        return 1
    print(f"{ours} replaces {there} on the channel")
    return 0


if __name__ == "__main__":
    sys.exit(main())
