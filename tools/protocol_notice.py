"""Open a release's notes with an update notice when its link protocol changed.

    python tools/protocol_notice.py --tag v0.2.0-beta.80 --notes notes.md

Stations of different link protocols cannot connect (each ignores the other's calls), so a
release that changes the protocol is not an update an operator may take later: everybody they
work has to move with them. Every installed version shows a release's notes in its updates
window — including versions too old to read the manifest's ``link_protocol`` — so the notice
goes at the top of the notes: the protocol at this tag against the previous ``v*`` tag's. The
release body and the updater manifest both carry the notes, so both say it.

Prints what it found; leaves the notes alone when the protocol is unchanged or the previous
release cannot be read.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

FRAMES = "core/aether-link/src/frames.rs"
PROTOCOL = re.compile(r"^pub const PROTOCOL_VERSION: u8 = (\d+);", re.MULTILINE)


def protocol_of(source: str) -> int | None:
    found = PROTOCOL.search(source)
    return int(found.group(1)) if found else None


def protocol_at(ref: str) -> int | None:
    try:
        source = subprocess.run(
            ["git", "show", f"{ref}:{FRAMES}"], capture_output=True, text=True, check=True
        ).stdout
    except subprocess.CalledProcessError:
        return None
    return protocol_of(source)


def previous_release(tag: str) -> str | None:
    """The newest ``v*`` tag other than ``tag`` that is an ancestor of it."""
    tags = subprocess.run(
        ["git", "tag", "--list", "v*", "--sort=-creatordate", "--merged", tag],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()
    return next((t for t in tags if t != tag), None)


def notice(version: str, ours: int, previous: str, theirs: int) -> str:
    return (
        f"> **Update required — link protocol {ours}.** {version} cannot connect to stations "
        f"running {previous} or earlier (link protocol {theirs}), and they cannot connect to "
        "it: each ignores the other's calls. Install this version, and ask the stations you "
        "work to update too.\n\n"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--tag", required=True, help="the tag being released")
    ap.add_argument("--notes", required=True, type=Path)
    args = ap.parse_args()
    ours = protocol_at(args.tag) or protocol_of(Path(FRAMES).read_text(encoding="utf-8"))
    previous = previous_release(args.tag)
    theirs = protocol_at(previous) if previous else None
    if ours is None or theirs is None or ours == theirs:
        print(f"link protocol {ours} at {args.tag}, {theirs} at {previous}: no notice")
        return 0
    version = args.tag.removeprefix("v")
    body = args.notes.read_text(encoding="utf-8") if args.notes.exists() else ""
    args.notes.write_text(notice(version, ours, previous, theirs) + body, encoding="utf-8")
    print(f"link protocol {theirs} at {previous}, {ours} at {args.tag}: update notice added")
    return 0


if __name__ == "__main__":
    sys.exit(main())
