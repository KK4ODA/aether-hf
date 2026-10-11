"""Run this build against the last release's daemon, both ways round (the nightly's ``mixed``).

    python tools/mixed_pair.py --daemon core/target/release/aetherd --out runs/mixed
                               [--tag v0.2.0-beta.104] [--old PATH] [--repo OWNER/NAME]

The stations on the air run released builds, and an operator updates when they get round to
it: a change that works between two copies of this tree can still fail against the release
everybody else has. This takes the newest ``v*`` tag (or ``--tag``), downloads that release's
Linux daemon from its release page (or uses ``--old``), and runs the scenarios tagged
``mixed`` twice through ``session_matrix.py``: this build calling the release, then the
release calling this build (``--daemon`` / ``--daemon-b``; a gateway scenario's clients run
``--daemon``). Each direction's runs and ``results.csv`` are under ``--out``.

When the link protocol changed since that release (``tools/protocol_notice.py``), the two
cannot connect by design and the release notes say *Update required*; nothing is run, and
``note.md`` says why. Exit status 1 when any run failed.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from protocol_notice import FRAMES, protocol_at, protocol_of

ROOT = Path(__file__).resolve().parents[1]
TRIPLE = "x86_64-unknown-linux-gnu"


def newest_tag() -> str | None:
    tags = subprocess.run(
        ["git", "tag", "--list", "v*", "--sort=-creatordate"],
        capture_output=True,
        text=True,
        check=False,
        cwd=ROOT,
    ).stdout.split()
    return tags[0] if tags else None


def fetch(tag: str, repo: str, into: Path) -> Path:
    """The release's Linux daemon, downloaded from its public release page and unpacked under
    ``into``."""
    into.mkdir(parents=True, exist_ok=True)
    name = f"aetherd-{tag.removeprefix('v')}-{TRIPLE}.tar.gz"
    archive = into / name
    url = f"https://github.com/{repo}/releases/download/{tag}/{name}"
    with urllib.request.urlopen(url, timeout=120) as response:
        archive.write_bytes(response.read())
    with tarfile.open(archive) as tar:
        tar.extractall(into, filter="data")
    daemon = next(p for p in into.rglob("aetherd") if p.is_file())
    daemon.chmod(0o755)
    return daemon


def run(caller: Path, called: Path, out: Path) -> int:
    out.mkdir(parents=True, exist_ok=True)
    return subprocess.run(
        [sys.executable, str(ROOT / "tools" / "session_matrix.py"), "--tag", "mixed",
         "--jobs", "2", "--drop-passing-audio", "--daemon", str(caller),
         "--daemon-b", str(called), "--out", str(out), "--csv", str(out / "results.csv")],
        check=False,
    ).returncode  # fmt: skip


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--daemon", type=Path, required=True, help="this build")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--tag", help="the release to run against (default: the newest v* tag)")
    ap.add_argument("--old", type=Path, help="that release's daemon, already at hand")
    ap.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "KK4ODA/aether-hf"))
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    # a shallow or fresh clone may have no tags: the release and its protocol are read from them
    subprocess.run(["git", "fetch", "--tags", "--quiet"], cwd=ROOT, check=False)
    tag = args.tag or newest_tag()
    if tag is None:
        (args.out / "note.md").write_text("no v* tag to run against.\n", encoding="utf-8")
        return 0
    ours = protocol_of((ROOT / FRAMES).read_text(encoding="utf-8"))
    theirs = protocol_at(tag)
    if theirs is None:
        print(f"{tag}'s link protocol could not be read; running anyway", flush=True)
    elif ours != theirs:
        note = (
            f"skipped: this build speaks link protocol {ours} and {tag} speaks {theirs}; they "
            "cannot connect by design, and the next release's notes will say Update required.\n"
        )
        (args.out / "note.md").write_text(note, encoding="utf-8")
        print(note.strip())
        return 0
    old = args.old or fetch(tag, args.repo, args.out / "release")
    short = tag.removeprefix("v")
    print(f"this build against {tag} ({old})", flush=True)
    calling = run(args.daemon, old, args.out / f"this-calls-{short}")
    called = run(old, args.daemon, args.out / f"{short}-calls-this")
    return 1 if calling or called else 0


if __name__ == "__main__":
    raise SystemExit(main())
