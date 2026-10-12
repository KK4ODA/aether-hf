"""List and download stations' uploads from the project's Drive folder, keeping them private.

    python tools/fetch_uploads.py list [--days 7]
    python tools/fetch_uploads.py get NAME-OR-ID [...] [--out field/inbox] [--extract]

Stations send their files with *Send my files to the Aether project*; they land in the owner's
"Aether HF uploads" folder through the upload script (``tools/drive_upload/Code.gs``). The files
stay private there. This reads them back through the same script, which answers ``list`` and
``read`` only to a request carrying the read key: the long random secret ``makeReadKey`` makes
in the script's properties, given to this tool as the environment variable
``AETHER_UPLOAD_READ_KEY``. Without it nothing can be read back, and with it only the files in
that folder. ``revokeReadKey`` in the script turns reading back off.

``get`` takes a file id, or any part of a name ``list`` shows (``KQ4SAX``, ``20261012``); a
part that matches several files takes them all. Each file comes in 8 MB pieces, is checked
against the size Drive reports, and is skipped when already downloaded whole. ``--extract``
unpacks each zip beside it into a folder of the same name.

The script's address is the one built into the program (``upload::PROJECT_ENDPOINT``);
``AETHER_UPLOAD_URL`` overrides it. Only the standard library: nothing to install.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENDPOINT = (
    "https://script.google.com/macros/s/"
    "AKfycbw5KhON9utE7jinNxv4g0ndw3k9wyw9miUGAZwPqLhoqh2tNZDBst8CiewQc5Q87avCIg/exec"
)
PIECE = 8 * 1024 * 1024
"""The script's own limit for one ``read`` (``READ_PIECE`` in Code.gs)."""


def ask(payload: dict[str, object]) -> dict:
    """POST as text, as the stations do; urllib follows the script's redirect to its answer."""
    key = os.environ.get("AETHER_UPLOAD_READ_KEY", "").strip()
    if not key:
        sys.exit(
            "AETHER_UPLOAD_READ_KEY is not set: run makeReadKey in the upload script's editor "
            "and add the key to this environment's settings."
        )
    request = urllib.request.Request(
        os.environ.get("AETHER_UPLOAD_URL", ENDPOINT),
        data=json.dumps({**payload, "key": key}).encode(),
        headers={"Content-Type": "text/plain; charset=utf-8"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=180) as response:
        text = response.read().decode(errors="replace")
    try:
        answer: dict = json.loads(text)
    except json.JSONDecodeError:
        sys.exit(
            "The upload script answered with a web page, not JSON: is the new version deployed?"
        )
    if answer.get("ok") is not True:
        sys.exit(f"The upload script refused: {answer.get('error')}")
    return answer


def listing(days: int) -> list[dict]:
    return list(ask({"action": "list", "days": days}).get("files", []))


def matching(wanted: str, files: list[dict]) -> list[dict]:
    exact = [f for f in files if f["id"] == wanted]
    return exact or [f for f in files if wanted.lower() in f["name"].lower()]


def download(file: dict, out: Path) -> Path:
    path = out / file["name"].replace("/", "_")
    size = int(file["size"])
    if path.exists() and path.stat().st_size == size:
        print(f"have {path.name} ({size / 1e6:.1f} MB)")
        return path
    part = path.with_name(path.name + ".part")
    with part.open("wb") as f:
        offset = 0
        while offset < size:
            answer = ask({"action": "read", "id": file["id"], "offset": offset, "length": PIECE})
            data = base64.b64decode(answer.get("data", ""))
            if not data:
                sys.exit(f"{file['name']}: no data at {offset} of {size}")
            f.write(data)
            offset += len(data)
            print(f"  {path.name}: {offset / 1e6:.1f} of {size / 1e6:.1f} MB", flush=True)
    if part.stat().st_size != size:
        sys.exit(f"{file['name']}: got {part.stat().st_size} bytes, Drive says {size}")
    part.replace(path)
    print(f"got {path.name}")
    return path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="command", required=True)
    ls = sub.add_parser("list", help="the uploads of the last days")
    ls.add_argument("--days", type=int, default=7)
    get = sub.add_parser("get", help="download uploads by id or part of their name")
    get.add_argument("wanted", nargs="+")
    get.add_argument("--days", type=int, default=30, help="how far back names are looked up")
    get.add_argument("--out", type=Path, default=ROOT / "field" / "inbox")
    get.add_argument("--extract", action="store_true", help="unpack each zip beside it")
    args = ap.parse_args()
    files = listing(args.days)
    if args.command == "list":
        for f in files:
            print(f"{f['created'][:16]}  {int(f['size']) / 1e6:6.1f} MB  {f['id']}  {f['name']}")
            if f.get("description"):
                print(f"{'':20}{f['description']}")
        return 0
    args.out.mkdir(parents=True, exist_ok=True)
    for wanted in args.wanted:
        found = matching(wanted, files)
        if not found:
            print(f"nothing in the last {args.days} days matches {wanted!r}", file=sys.stderr)
            return 1
        for f in found:
            path = download(f, args.out)
            if args.extract and path.suffix == ".zip":
                into = path.with_suffix("")
                with zipfile.ZipFile(path) as z:
                    z.extractall(into)
                print(f"unpacked into {into}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
