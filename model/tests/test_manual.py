"""The operator's manual must not drift from the application it describes.

``docs/manual/manual.md`` is part of the application (CONTRIBUTING rule 12). Its contents, its
rung tables and its schematics are made by tools, and the rung tables and two of the schematics
read the reference model, so a change to the waveform that left the manual behind fails here, as
a stale air-interface specification fails ``test_spec.py``. The screenshots are taken by hand
(``node tools/manual/screens.mjs``) and are checked only to exist and to be used.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "tools" / "manual" / "build.py"
FIGURES = ROOT / "tools" / "manual" / "figures.py"


def run(tool: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(tool), *args],
        capture_output=True,
        text=True,
        cwd=ROOT,
        encoding="utf-8",
    )


def test_the_manual_agrees_with_itself_and_the_model() -> None:
    result = run(BUILD, "check")
    assert result.returncode == 0, (
        "docs/manual/manual.md is out of date.\n"
        "Run: python -m uv run python tools/manual/build.py update\n"
        f"{result.stdout}{result.stderr}"
    )


def test_the_schematics_are_what_their_script_draws() -> None:
    result = run(FIGURES, "--check")
    assert result.returncode == 0, (
        "docs/manual/fig/ is out of date with tools/manual/figures.py.\n"
        "Run: python -m uv run python tools/manual/figures.py\n"
        f"{result.stdout}{result.stderr}"
    )


def test_the_printable_page_builds_without_a_browser() -> None:
    """The HTML the PDF is printed from needs only the standard library and the files."""
    sys.path.insert(0, str(BUILD.parent))
    try:
        import build

        page = build.page_html(build.read(build.MANUAL))
    finally:
        sys.path.remove(str(BUILD.parent))
    assert page.startswith("<!doctype html>")
    assert "Aether operator's manual" in page
    # every picture travels inside the page, so the PDF carries it
    assert 'src="fig/' not in page and 'src="screens/' not in page
    # links into the repository are written as addresses a printed page can follow
    assert 'href="../' not in page
