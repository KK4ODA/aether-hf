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
against the size Drive reports, and is skipped when already downloaded whole. A piece the script
answers with no data, or whose connection drops, is asked for again (``TRIES``), and a download
that stopped carries on from the end of its ``.part`` file. ``--extract``
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
import time
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
TRIES = 6
"""How often a request is made before giving up. The script sometimes answers a ``read`` with
no data, or the connection is reset mid-answer; asking again a little later gets it."""


class Unanswered(RuntimeError):
    """The script answered with something other than JSON: a passing fault, worth asking again."""


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
        raise Unanswered(
            "The upload script answered with a web page, not JSON: is the new version deployed?"
        ) from None
    if answer.get("ok") is not True:
        sys.exit(f"The upload script refused: {answer.get('error')}")
    return answer


def asked_again(payload: dict[str, object], what: str, wait_s: float = 5.0) -> dict:
    """:func:`ask`, asked again after a dropped connection or an answer that is not JSON.
    The script answers one request at a time: two downloads at once get 404s and empty
    listings, so run one ``get`` at a time."""
    for attempt in range(1, TRIES + 1):
        try:
            return ask(payload)
        except (OSError, Unanswered) as error:
            why = f"{type(error).__name__}: {error}"
        if attempt < TRIES:
            print(f"  {what}: {why}; asking again", flush=True)
            time.sleep(wait_s * attempt)
    sys.exit(f"{what}: {why}, {TRIES} times")


def listing(days: int) -> list[dict]:
    return list(asked_again({"action": "list", "days": days}, "the listing").get("files", []))


def matching(wanted: str, files: list[dict]) -> list[dict]:
    exact = [f for f in files if f["id"] == wanted]
    return exact or [f for f in files if wanted.lower() in f["name"].lower()]


def piece(file: dict, offset: int, length: int, wait_s: float = 5.0) -> bytes:
    """One piece of a file, asked for again after an empty answer or a dropped connection."""
    for attempt in range(1, TRIES + 1):
        try:
            answer = ask({"action": "read", "id": file["id"], "offset": offset, "length": length})
            data = base64.b64decode(answer.get("data", ""))
            if data:
                return data
            why = "no data"
        except (OSError, Unanswered) as error:
            why = f"{type(error).__name__}: {error}"
        if attempt < TRIES:
            print(f"  {file['name']}: {why} at {offset}; asking again", flush=True)
            time.sleep(wait_s * attempt)
    sys.exit(f"{file['name']}: {why} at {offset}, {TRIES} times")


def download(file: dict, out: Path, wait_s: float = 5.0) -> Path:
    path = out / file["name"].replace("/", "_")
    size = int(file["size"])
    if path.exists() and path.stat().st_size == size:
        print(f"have {path.name} ({size / 1e6:.1f} MB)")
        return path
    part = path.with_name(path.name + ".part")
    # A .part left by a run that stopped holds the file's first bytes: carry on from its end.
    offset = part.stat().st_size if part.exists() and part.stat().st_size <= size else 0
    with part.open("ab" if offset else "wb") as f:
        if offset:
            print(f"  {path.name}: resuming at {offset / 1e6:.1f} MB", flush=True)
        while offset < size:
            data = piece(file, offset, min(PIECE, size - offset), wait_s)
            f.write(data)
            f.flush()
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
