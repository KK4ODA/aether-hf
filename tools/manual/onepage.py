# SPDX-License-Identifier: MIT OR Apache-2.0
"""The one-page guides: Aether with VarAC, and Aether with Winlink Express, as PDFs to print.

    python tools/manual/onepage.py print     # print each guide to its PDF, beside its source
    python tools/manual/onepage.py check     # each PDF is one page, printed from its source (CI)

Each guide is Markdown in docs/user/ (it reads on GitHub as it is), and its PDF sits beside it,
committed, so the README can link it. `print` renders the Markdown with the manual's converter
(build.py), prints it through Chrome or Chromium as build.py's `pdf` does, refuses a guide that
runs onto a second page, and writes the SHA-256 of each source into one-page.json. `check` fails
when a source has changed since its PDF was printed, or a PDF is missing or longer than a page:
a change to a guide is printed again in the same commit, as a change to the manual updates it.
"""

from __future__ import annotations

import hashlib
import html
import json
import re
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import build

USER = build.ROOT / "docs" / "user"
GUIDES = ("one-page-varac", "one-page-winlink-express")
RECORD = USER / "one-page.json"

STYLE = """
@page { size: letter; margin: 0.45in 0.5in 0.55in;
  @bottom-left { content: "Aether HF · github.com/KK4ODA/aether-hf · CC BY 4.0";
                 font: 7.5pt Inter, sans-serif; color: #94a3b8; }
  @bottom-right { content: "MADE"; font: 7.5pt Inter, sans-serif; color: #94a3b8; } }
:root { --ink: #1e293b; --muted: #475569; --hair: #cbd5e1; --accent: #0f766e; }
html { font: 9pt/1.38 Inter, "Segoe UI", "Helvetica Neue", Arial, sans-serif; color: var(--ink); }
body { margin: 0; }
h1 { font-size: 19pt; margin: 0 0 0.15rem; color: var(--accent); }
h1 + p { margin: 0 0 0.6rem; color: var(--muted); }
.columns { columns: 2; column-gap: 0.32in; }
h2 { font-size: 10.5pt; margin: 0.55rem 0 0.2rem; break-after: avoid; color: var(--accent); }
.columns > h2:first-child { margin-top: 0; }
p { margin: 0.2rem 0; }
ul, ol { margin: 0.15rem 0; padding-left: 1.2rem; }
li { margin: 0.12rem 0; break-inside: avoid; }
a { color: var(--accent); text-decoration: none; }
code { font: 8.2pt "JetBrains Mono", Consolas, "DejaVu Sans Mono", monospace;
       background: #f1f5f9; padding: 0 0.15em; border-radius: 2px; white-space: nowrap; }
.trouble h2 { margin-top: 0.7rem; }
table { border-collapse: collapse; width: 100%; font-size: 8.7pt; }
th, td { border-bottom: 1px solid var(--hair); padding: 0.18rem 0.4rem; text-align: left;
         vertical-align: top; }
th { background: #f1f5f9; }
td:first-child { width: 31%; }
"""


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def page_html(source: Path) -> str:
    """The guide as one page: the title and its line, two columns, the table across the foot."""
    text = build.read(source)
    # links in the guide are relative to docs/user, as GitHub reads them; build.absolute makes
    # them the repository's addresses from wherever build.MANUAL says the page is
    build.MANUAL = source
    lines = text.split("\n")
    sections = [i for i, line in enumerate(lines) if line.startswith("## ")]
    first, last = sections[0], sections[-1]
    head, body, trouble = lines[:first], lines[first:last], lines[last:]
    made = f"printed {datetime.now(UTC):%Y-%m-%d} from docs/user/{source.name}"
    title = build.HEADING.match(head[0])
    name = title.group(2) if title else source.stem
    return (
        '<!doctype html>\n<html lang="en"><head><meta charset="utf-8">'
        f"<title>{html.escape(name)}</title>"
        f"<style>{STYLE.replace('MADE', made)}</style></head><body>"
        f"{build.blocks(head, build.Page())}"
        f'<div class="columns">{build.blocks(body, build.Page())}</div>'
        f'<div class="trouble">{build.blocks(trouble, build.Page())}</div>'
        "</body></html>\n"
    )


def pages_in(pdf: bytes) -> int:
    """The pages of a PDF Chrome wrote: its page objects, which it never compresses."""
    return len(re.findall(rb"/Type\s*/Page(?![s\w])", pdf))


def print_all() -> list[str]:
    record: dict[str, str] = {}
    problems: list[str] = []
    for guide in GUIDES:
        source, out = USER / f"{guide}.md", USER / f"{guide}.pdf"
        with tempfile.TemporaryDirectory() as tmp:
            pdf = Path(tmp) / out.name
            build.write_pdf(page_html(source), pdf)
            pages = pages_in(pdf.read_bytes())
            if pages != 1:
                problems.append(f"{source.name} prints on {pages} pages: shorten it to one")
                continue
            out.write_bytes(pdf.read_bytes())
        record[source.name] = digest(source)
        print(f"{out.relative_to(build.ROOT)}: one page")
    if not problems:
        RECORD.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    return problems


def check() -> list[str]:
    record = json.loads(RECORD.read_text(encoding="utf-8")) if RECORD.exists() else {}
    problems: list[str] = []
    for guide in GUIDES:
        source, pdf = USER / f"{guide}.md", USER / f"{guide}.pdf"
        problems += build.link_problems(build.read(source), source)
        if not pdf.exists():
            problems.append(f"{pdf.name} is missing")
        elif pages_in(pdf.read_bytes()) != 1:
            problems.append(f"{pdf.name} is not one page")
        if record.get(source.name) != digest(source):
            problems.append(f"{source.name} changed since {pdf.name} was printed")
    return problems


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else "check"
    if command == "print":
        problems = print_all()
    elif command == "check":
        problems = check()
    else:
        print(__doc__)
        return 2
    for problem in problems:
        print(problem)
    if problems and command == "check":
        print("Run: python tools/manual/onepage.py print")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
