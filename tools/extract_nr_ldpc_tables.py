"""Extract the 3GPP TS 38.212 LDPC base-graph shift tables from the published spec.

Usage:
    python tools/extract_nr_ldpc_tables.py [--version h00] [--out model/aether_model/fec/data/nr_ldpc_base_graphs.json]

Downloads ``38212-<version>.zip`` from the 3GPP specification archive (public), reads the
.docx inside with python-docx, locates Tables 5.3.2-2 (BG1) and 5.3.2-3 (BG2) by their
headers, and writes one JSON file:

    {"source": ..., "bg1": {"rows": 46, "cols": 68, "entries": [[i, j, [v0..v7]], ...]},
     "bg2": {"rows": 42, "cols": 52, "entries": [...]}}

The output is committed to the repository so normal builds never need network access or
python-docx; re-run this tool only to refresh from a newer spec version (the tables have
been stable since Rel-15).

Requires: pip install python-docx (only for running this tool).
"""

from __future__ import annotations

import argparse
import io
import json
import sys
import urllib.request
import zipfile
from pathlib import Path

ARCHIVE = "https://www.3gpp.org/ftp/Specs/archive/38_series/38.212/38212-{version}.zip"
EXPECTED = {"bg1": (46, 68, 316), "bg2": (42, 52, 197)}


def _fetch_docx(version: str) -> bytes:
    url = ARCHIVE.format(version=version)
    req = urllib.request.Request(
        url, headers={"User-Agent": "aether-hf tools/extract_nr_ldpc_tables"}
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = resp.read()
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        names = [n for n in z.namelist() if n.lower().endswith(".docx")]
        if len(names) != 1:
            raise SystemExit(f"expected one .docx in {url}, found {names}")
        return z.read(names[0])


def _parse_shift_table(table) -> list[tuple[int, int, list[int]]]:  # type: ignore[no-untyped-def]
    """Rows are laid out as two side-by-side groups: (i, j, v0..v7 | i, j, v0..v7)."""
    entries: list[tuple[int, int, list[int]]] = []
    for row in table.rows:
        cells = [c.text.strip() for c in row.cells]
        for group in (cells[0:10], cells[10:20]):
            if len(group) < 10 or not group[0].isdigit() or not group[1].isdigit():
                continue
            shifts = group[2:10]
            if not all(s.lstrip("-").isdigit() for s in shifts):
                continue
            entries.append((int(group[0]), int(group[1]), [int(s) for s in shifts]))
    return entries


def _find_tables(doc):  # type: ignore[no-untyped-def]
    found = []
    for t in doc.tables:
        if len(t.rows) < 50 or len(t.columns) != 20:
            continue
        header = " ".join(" ".join(c.text.split()) for c in t.rows[1].cells)
        if "Row index" in header and "Column index" in header and "Set index" in header:
            found.append(t)
    return found


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--version", default="h00", help="3GPP archive version tag, e.g. h00 = V17.0.0")
    ap.add_argument("--out", default="model/aether_model/fec/data/nr_ldpc_base_graphs.json")
    args = ap.parse_args()

    try:
        import docx  # type: ignore[import-not-found]
    except ImportError:
        print("python-docx is required: pip install python-docx", file=sys.stderr)
        return 2

    blob = _fetch_docx(args.version)
    doc = docx.Document(io.BytesIO(blob))
    tables = _find_tables(doc)
    if len(tables) != 2:
        raise SystemExit(f"expected exactly 2 shift tables, found {len(tables)}")

    out: dict[str, object] = {
        "source": f"3GPP TS 38.212 V{args.version} Tables 5.3.2-2 (BG1) and 5.3.2-3 (BG2), "
        f"extracted from {ARCHIVE.format(version=args.version)}",
        "set_index_note": "Vi,j for set index iLS = 0..7; shift = Vi,j mod Z (TS 38.212 5.3.2)",
        "lifting_sets": {
            "0": [2, 4, 8, 16, 32, 64, 128, 256],
            "1": [3, 6, 12, 24, 48, 96, 192, 384],
            "2": [5, 10, 20, 40, 80, 160, 320],
            "3": [7, 14, 28, 56, 112, 224],
            "4": [9, 18, 36, 72, 144, 288],
            "5": [11, 22, 44, 88, 176, 352],
            "6": [13, 26, 52, 104, 208],
            "7": [15, 30, 60, 120, 240],
        },
    }
    for name, table in zip(("bg1", "bg2"), tables, strict=True):
        entries = _parse_shift_table(table)
        rows = max(i for i, _, _ in entries) + 1
        cols = max(j for _, j, _ in entries) + 1
        exp_rows, exp_cols, exp_n = EXPECTED[name]
        if (rows, cols, len(entries)) != (exp_rows, exp_cols, exp_n):
            raise SystemExit(
                f"{name}: parsed {rows}x{cols} with {len(entries)} entries, "
                f"expected {exp_rows}x{exp_cols} with {exp_n}"
            )
        if len({(i, j) for i, j, _ in entries}) != len(entries):
            raise SystemExit(f"{name}: duplicate (i, j) entries")
        out[name] = {"rows": rows, "cols": cols, "entries": [[i, j, v] for i, j, v in entries]}
        print(f"{name}: {rows}x{cols}, {len(entries)} entries")

    path = Path(args.out)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {path} ({path.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
