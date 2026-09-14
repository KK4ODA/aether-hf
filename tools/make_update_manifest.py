"""Write the updater manifest the desktop shell checks for new versions.

    python tools/make_update_manifest.py --version 0.3.0 --url-base URL --assets DIR
                                         [--notes FILE] [--out latest.json]

The Tauri updater fetches one JSON document per channel, verifies each platform entry's
Ed25519 signature against the public key built into the app, downloads the matching
installer and runs it. This writes that document from a directory of release assets: every
installer that has a `.sig` beside it (the bundler writes one when the signing key is
present at build time) becomes a platform entry pointing at `<url-base>/<file name>`.

Written here rather than by a third-party action so the release workflow has one fewer
black box: the format is five keys and a table, and this is the whole of it.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import UTC, datetime
from pathlib import Path

# how an asset's name maps to the platform key the updater asks for
PLATFORMS: list[tuple[str, str]] = [
    ("x64-setup.exe", "windows-x86_64"),
    ("arm64-setup.exe", "windows-aarch64"),
    ("amd64.AppImage", "linux-x86_64"),
    ("aarch64.AppImage", "linux-aarch64"),
    ("arm64.AppImage", "linux-aarch64"),
    ("x64.app.tar.gz", "darwin-x86_64"),
    ("aarch64.app.tar.gz", "darwin-aarch64"),
]


def platform_of(name: str) -> str | None:
    for suffix, platform in PLATFORMS:
        if name.endswith(suffix):
            return platform
    return None


def build(version: str, url_base: str, assets: Path, notes: str) -> dict[str, object]:
    platforms: dict[str, dict[str, str]] = {}
    for signature in sorted(assets.glob("*.sig")):
        asset = signature.with_suffix("")  # foo.exe.sig -> foo.exe
        platform = platform_of(asset.name)
        if platform is None or not asset.is_file():
            continue
        platforms[platform] = {
            "signature": signature.read_text(encoding="utf-8").strip(),
            "url": f"{url_base.rstrip('/')}/{asset.name}",
        }
    return {
        "version": version,
        "notes": notes,
        "pub_date": datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "platforms": platforms,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--url-base", required=True, help="where the assets are downloaded from")
    parser.add_argument("--assets", required=True, type=Path, help="directory of release assets")
    parser.add_argument("--notes", type=Path, help="release notes, shown by the updater")
    parser.add_argument("--out", type=Path, default=Path("latest.json"))
    args = parser.parse_args()

    notes = args.notes.read_text(encoding="utf-8").strip() if args.notes else ""
    manifest = build(args.version, args.url_base, args.assets, notes)
    if not manifest["platforms"]:
        print("no signed installers found; nothing for the updater to offer", file=sys.stderr)
        return 1
    args.out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    platforms = manifest["platforms"]
    assert isinstance(platforms, dict)
    print(f"{args.out}: {', '.join(sorted(platforms))}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
