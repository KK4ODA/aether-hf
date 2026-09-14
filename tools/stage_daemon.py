"""Put the daemon where the desktop shell's bundler expects it.

    python tools/stage_daemon.py [--target <triple>] [--from <path/to/aetherd[.exe]>] [--no-build]

Tauri bundles a sidecar from `app/src-tauri/binaries/<name>-<target-triple>[.exe]` and
installs it beside the shell's own binary with the triple stripped, which is where the shell
looks for it first. Cargo puts the daemon in `core/target/<triple>/release/` (or
`core/target/release/` for a native build), so something has to copy it across and name it
correctly, and that something should not be a person remembering the host triple.

Without `--from` the daemon is built in release mode first; `--no-build` skips that when a
build is already there. `--target` defaults to the host, as `rustc -vV` reports it.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "core"
BINARIES = ROOT / "app" / "src-tauri" / "binaries"


def host_triple() -> str:
    out = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
    for line in out.splitlines():
        if line.startswith("host:"):
            return line.split(":", 1)[1].strip()
    sys.exit("rustc -vV did not report a host triple")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", help="target triple (default: the host)")
    parser.add_argument("--from", dest="source", help="an already-built daemon to stage")
    parser.add_argument("--no-build", action="store_true", help="do not build the daemon")
    args = parser.parse_args()

    target = args.target or host_triple()
    suffix = ".exe" if "windows" in target else ""

    if args.source:
        source = Path(args.source)
    else:
        if not args.no_build:
            command = ["cargo", "build", "--release", "-p", "aetherd"]
            if args.target:
                command += ["--target", target]
            subprocess.run(command, cwd=CORE, check=True)
        out_dir = CORE / "target" / (target if args.target else "") / "release"
        source = out_dir / f"aetherd{suffix}"
    if not source.is_file():
        sys.exit(f"no daemon at {source}")

    BINARIES.mkdir(parents=True, exist_ok=True)
    destination = BINARIES / f"aetherd-{target}{suffix}"
    shutil.copy2(source, destination)
    print(f"{source} -> {destination.relative_to(ROOT).as_posix()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
