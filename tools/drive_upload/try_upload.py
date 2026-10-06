"""Try the upload script once deployed: send a small file the way an Aether station does.

    python tools/drive_upload/try_upload.py https://script.google.com/macros/s/…/exec CODE

It asks the script for a place to put one file (``begin``), sends a 1 MB test zip there in two
pieces through Google Drive's resumable upload — the second piece after asking the session
where it stands, as a station does after a dropped connection — and tells the script it is
done (``finish``). If it prints the file's link, the script works: the file is in your
"Aether HF uploads" folder, an email is on its way to you, and the code has one upload fewer.

Only the standard library: nothing to install.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

PIECE = 256 * 1024 * 2  # a multiple of 256 KiB, as every piece but the last must be


def ask(endpoint: str, payload: dict[str, object]) -> dict[str, object]:
    """POST the request as text; urllib follows the script's redirect to its answer."""
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "text/plain; charset=utf-8"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        answer: dict[str, object] = json.loads(response.read().decode())
    if answer.get("ok") is not True:
        sys.exit(f"The script refused: {answer.get('error')}")
    return answer


def put(url: str, content_range: str, body: bytes) -> tuple[int, dict[str, str], bytes]:
    """PUT a piece; a 308 (Resume Incomplete) is an answer here, not a redirect to follow."""
    request = urllib.request.Request(
        url, data=body, headers={"Content-Range": content_range}, method="PUT"
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read()


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("endpoint", help="the web app's address, ending /exec")
    ap.add_argument("code", help="a code makeCodes made")
    args = ap.parse_args()

    data = bytes(i * 7 % 251 for i in range(1024 * 1024))
    total = len(data)
    begun = ask(
        args.endpoint,
        {
            "action": "begin",
            "code": args.code,
            "name": "try_upload.zip",
            "size": total,
            "callsign": "TEST",
            "version": "try_upload",
        },
    )
    session = str(begun["upload_url"])
    print(f"begin: a place for {total} bytes, for {begun.get('to')}")

    status, headers, _ = put(session, f"bytes 0-{PIECE - 1}/{total}", data[:PIECE])
    print(f"first piece: {status} {headers.get('Range', '')}")
    status, headers, _ = put(session, f"bytes */{total}", b"")
    held = headers.get("Range", "")
    start = int(held.rsplit("-", 1)[1]) + 1 if held else 0
    print(f"where it stands: {status} {held} -> carry on from {start}")
    status, _, body = put(session, f"bytes {start}-{total - 1}/{total}", data[start:])
    if status not in (200, 201):
        sys.exit(f"The upload ended {status}: {body[:300]!r}")
    file_id = json.loads(body.decode())["id"]
    print(f"uploaded: file {file_id}")

    finished = ask(
        args.endpoint,
        {
            "action": "finish",
            "code": args.code,
            "file_id": file_id,
            "name": "try_upload.zip",
            "size": total,
            "callsign": "TEST",
            "note": "try_upload.py: the upload script works.",
        },
    )
    print(f"finish: {finished.get('link')}")


if __name__ == "__main__":
    main()
