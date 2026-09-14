"""The one version number, and the places it has to agree.

    python tools/release.py check [--tag vX.Y.Z]
    python tools/release.py bump X.Y.Z[-beta.N]

A release is cut from a tag, and everything the tag builds — the daemon, the desktop shell,
the installer, the reference model — has to say the same version, or a bug report saying
"0.3.0" means nothing. The number lives in five files; `check` fails when they disagree
(CI runs it, and the release workflow runs it against the tag), and `bump` rewrites all of
them and refreshes the lockfiles so a bump is one commit with nothing left to remember.

Why not derive the others from one file at build time: the Tauri bundler, Cargo and the
Python packaging each read their own file, and a generated file that is not committed is a
build that cannot be reproduced from a checkout.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-(?:alpha|beta|rc)\.\d+)?$")

# (file, pattern with one group around the version, how to read it back)
SITES: list[tuple[Path, str]] = [
    (
        ROOT / "core" / "Cargo.toml",
        r'(?m)^(?P<pre>\[workspace\.package\]\nversion = ")(?P<v>[^"]+)(?P<post>")',
    ),
    (
        ROOT / "app" / "src-tauri" / "Cargo.toml",
        r'(?m)^(?P<pre>version = ")(?P<v>[^"]+)(?P<post>")',
    ),
    (
        ROOT / "app" / "src-tauri" / "tauri.conf.json",
        r'(?P<pre>"version": ")(?P<v>[^"]+)(?P<post>")',
    ),
    (ROOT / "pyproject.toml", r'(?m)^(?P<pre>version = ")(?P<v>[^"]+)(?P<post>")'),
    (
        ROOT / "model" / "aether_model" / "__init__.py",
        r'(?m)^(?P<pre>__version__ = ")(?P<v>[^"]+)(?P<post>")',
    ),
]


def read_versions() -> dict[Path, str]:
    found: dict[Path, str] = {}
    for path, pattern in SITES:
        text = path.read_text(encoding="utf-8")
        match = re.search(pattern, text)
        if match is None:
            sys.exit(f"{path.relative_to(ROOT)}: no version found")
        found[path] = match.group("v")
    return found


def check(tag: str | None) -> int:
    versions = read_versions()
    distinct = sorted(set(versions.values()))
    for path, version in versions.items():
        print(f"{version:>14}  {path.relative_to(ROOT).as_posix()}")
    if len(distinct) != 1:
        print(f"versions disagree: {', '.join(distinct)}; run `python tools/release.py bump X.Y.Z`")
        return 1
    version = distinct[0]
    if tag is not None:
        expected = f"v{version}"
        if tag != expected:
            print(f"the tag is {tag} but the tree says {expected}")
            return 1
    print(f"ok: {version}")
    return 0


def bump(version: str) -> int:
    if not SEMVER.match(version):
        sys.exit(f"{version!r} is not a version like 0.3.0 or 0.3.0-beta.1")
    for path, pattern in SITES:
        text = path.read_text(encoding="utf-8")
        new, count = re.subn(pattern, rf"\g<pre>{version}\g<post>", text, count=1)
        if count != 1:
            sys.exit(f"{path.relative_to(ROOT)}: no version found")
        path.write_text(new, encoding="utf-8", newline="\n")
        print(f"{path.relative_to(ROOT).as_posix()} -> {version}")
    # Cargo.lock records the workspace's own versions; refresh them without touching the
    # dependency versions, and without the network
    for manifest_dir in (ROOT / "core", ROOT / "app" / "src-tauri"):
        subprocess.run(
            ["cargo", "update", "--workspace", "--offline"],
            cwd=manifest_dir,
            check=True,
        )
    # the tauri.conf.json schema field is checked by the bundler, so re-validate it parses
    json.loads((ROOT / "app" / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    check_p = sub.add_parser("check", help="every file agrees, and matches the tag if given")
    check_p.add_argument("--tag", help="a git tag like v0.3.0 the tree must match")
    bump_p = sub.add_parser("bump", help="set the version everywhere")
    bump_p.add_argument("version")
    args = parser.parse_args()
    if args.command == "check":
        return check(args.tag)
    return bump(args.version)


if __name__ == "__main__":
    sys.exit(main())
