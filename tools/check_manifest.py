"""Every URL an updater manifest names must already resolve.

    python tools/check_manifest.py dist/latest.json

A manifest is a promise: an installation that reads it will try to download exactly the
URLs it lists. Publishing one whose assets are missing offers an update that cannot be
taken — which is worse than offering nothing, because the installation reports a failure
the operator cannot act on. A release that has lost an asset to a flaky upload is caught
here rather than by whoever updates next.

Exits non-zero, naming every platform that does not resolve.
"""

from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

TIMEOUT_S = 30


def resolves(url: str) -> str | None:
    """`None` when the URL is there, else why it is not."""
    try:
        with urllib.request.urlopen(
            urllib.request.Request(url, method="HEAD"), timeout=TIMEOUT_S
        ) as answer:
            if answer.status != 200:
                return f"HTTP {answer.status}"
    except (urllib.error.URLError, OSError, ValueError) as error:
        return str(error)
    return None


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    path = Path(sys.argv[1])
    manifest = json.loads(path.read_text(encoding="utf-8"))
    platforms = manifest.get("platforms", {})
    if not platforms:
        print(f"::error::{path} names no platforms")
        return 1

    bad: list[str] = []
    for name, platform in sorted(platforms.items()):
        url = platform.get("url", "")
        why = resolves(url)
        print(f"  {name}: {'ok' if why is None else why}  {url.rsplit('/', 1)[-1]}")
        if why is not None:
            bad.append(f"{name} ({why})")

    if bad:
        print(f"::error::the manifest names assets that do not resolve: {'; '.join(bad)}")
        return 1
    print(f"{manifest.get('version')} is offered on {len(platforms)} platforms, all present")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
