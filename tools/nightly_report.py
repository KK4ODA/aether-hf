"""Open, update or close the GitHub issue that tracks a failing nightly run.

    python tools/nightly_report.py --runs runs/ --results "build=success full=success …"
                                   --run-url URL --sha SHA --repo OWNER/NAME [--dry-run]

The nightly's jobs fail only for what is serious, so that the author is emailed only for it
(the author's rule, 2026-11-14): the daemon not building or the full tests failing, a run that
broke an invariant (a daemon that died or panicked, a stuck key, a harness fault, a host told
what it must not be, an identifier out of place), a scenario that failed all of its draws, a
quarter of the scenarios slower or failing at once, a soak run that broke an invariant, or this
build against the last release breaking an invariant or failing a scenario both ways round. One
draw of a hard path missing its time budget is a warning on a green run, and nothing here.

The email GitHub sends for a failed scheduled run says only that it failed. This reads what
the nightly's jobs left (``--runs``: the run's artifacts, one folder each — the scenario
variants' ``results.csv``, the trend table, ``soak.json``, the mixed pair's ``results.csv``
and ``serious.md``) and writes what failed and why into an issue labelled ``nightly``: a new
one when none is open, a comment on the open one otherwise, so a broken week is one thread. A
night with nothing serious opens nothing, and says so on an open issue as it closes it.

Uses the ``gh`` command line (on GitHub's runners) with the job's token; ``--dry-run`` prints
the text instead. Standard library only: the job that runs it installs nothing.
"""

from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path

LABEL = "nightly"
TITLE = "Nightly run failing"


def parse_results(text: str) -> dict[str, str]:
    """``"full=success scenarios=failure"`` → ``{"full": "success", …}``."""
    out = {}
    for word in text.split():
        job, _, result = word.partition("=")
        if job:
            out[job] = result
    return out


def failed_rows(path: Path, *, invariants: bool | None = None) -> list[str]:
    """The failing runs of a ``results.csv``, one Markdown line each: all of them, only those
    that broke an invariant (``True``), or only those that did not (``False``). A CSV written
    before the ``invariants`` column counts every failure as not one."""
    with path.open(newline="", encoding="utf-8") as f:
        return [
            f"- `{r['scenario']}`: {r.get('why') or 'failed'}"
            for r in csv.DictReader(f)
            if r.get("pass") != "True"
            and (invariants is None or bool(r.get("invariants")) == invariants)
        ]


def minor_rows(runs: Path) -> list[str]:
    """The scenario runs that failed without breaking an invariant: warnings, not serious."""
    lines: list[str] = []
    for csv_path in sorted(runs.glob("scenarios-*/results.csv")):
        lines += failed_rows(csv_path, invariants=False)
    return lines


def body(runs: Path, results: dict[str, str], run_url: str, sha: str, today: str) -> str:
    failed = [job for job, result in results.items() if result == "failure"]
    lines = [
        f"**{today}**, `{sha[:7]}`: {', '.join(failed) or 'nothing'} failed — [the run]({run_url}).",
        "",
    ]
    if "build" in failed:
        lines += ["**Build:** the daemon did not build, so nothing ran; the log says why.", ""]
    if "full" in failed:
        lines += ["**Tests:** the full pytest set failed; the run's log has the test.", ""]
    broke: list[str] = []
    for csv_path in sorted(runs.glob("scenarios-*/results.csv")):
        broke += failed_rows(csv_path, invariants=True)
    if broke:
        lines += ["**Scenarios that broke an invariant:**", *broke, ""]
    elif "scenarios" in failed:
        lines += [
            "**Scenarios:** the job failed with no run breaking an invariant — the runner "
            "itself, before or while writing its results; the job's log says why.",
            "",
        ]
    trend = runs / "trend" / "trend.md"
    if "trend" in failed and trend.exists():
        head = trend.read_text(encoding="utf-8").split("<details>")[0].strip()
        lines += ["**Speed against the baseline:**", "", head.replace("## ", "### "), ""]
    elif "trend" in failed:
        lines += ["**Speed against the baseline:** the comparison failed before its table.", ""]
    soak = runs / "soak" / "soak.json"
    if soak.exists():
        broken = [r for r in json.loads(soak.read_text(encoding="utf-8")) if not r.get("pass")]
        if broken:
            lines += [
                f"**Soak** ({len(broken)} broke an invariant; each one's TOML is in the run's "
                "`soak` artifact, and `session_matrix.py <it>` replays it):",
                *[f"- `{r['scenario']}`: {r.get('why')}" for r in broken],
                "",
            ]
    mixed = runs / "mixed"
    for name in ("note.md", "serious.md"):
        note = mixed / name
        if note.exists():
            lines += [
                "**Against the last release:** " + note.read_text(encoding="utf-8").strip(),
                "",
            ]
    mixed_lines: list[str] = []
    for csv_path in sorted(mixed.glob("*/results.csv")):
        mixed_lines += [
            f"{line} ({csv_path.parent.name}{', an invariant' if grave else ''})"
            for grave in (True, False)
            for line in failed_rows(csv_path, invariants=grave)
        ]
    if mixed_lines:
        lines += ["**This build with the last release:**", *mixed_lines, ""]
    minor = minor_rows(runs)
    if minor:
        lines += [
            f"<details><summary>Also tonight, not serious on its own ({len(minor)})</summary>",
            "",
            *minor,
            "",
            "</details>",
            "",
        ]
    lines.append("Closed by the first night with nothing serious.")
    return "\n".join(lines)


def quiet(runs: Path, run_url: str, sha: str, today: str) -> str:
    """What a night with nothing serious says on the open issue as it closes it."""
    minor = len(minor_rows(runs))
    also = (
        f" {minor} single-draw {'miss' if minor == 1 else 'misses'} stayed a warning on the run."
        if minor
        else ""
    )
    return f"Nothing serious on {today} at `{sha[:7]}` — [the run]({run_url}).{also} Closing."


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--runs", type=Path, required=True)
    ap.add_argument("--results", required=True)
    ap.add_argument("--run-url", required=True)
    ap.add_argument("--sha", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    results = parse_results(args.results)
    today = datetime.now(UTC).strftime("%Y-%m-%d")
    # the jobs fail only for what is serious (the module's docstring)
    failing = any(r == "failure" for r in results.values())
    text = (
        body(args.runs, results, args.run_url, args.sha, today)
        if failing
        else quiet(args.runs, args.run_url, args.sha, today)
    )
    if args.dry_run:
        print(text)
        return 0
    found = gh(
        "issue", "list", "--repo", args.repo, "--label", LABEL, "--state", "open",
        "--json", "number", "--jq", ".[0].number",
    ).strip()  # fmt: skip
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False, encoding="utf-8") as f:
        f.write(text + "\n")
    if failing and found:
        gh("issue", "comment", found, "--repo", args.repo, "--body-file", f.name)
        print(f"commented on #{found}")
    elif failing:
        subprocess.run(
            ["gh", "label", "create", LABEL, "--repo", args.repo, "--color", "B60205",
             "--description", "The nightly run is failing"],
            capture_output=True, check=False,
        )  # fmt: skip
        out = gh(
            "issue", "create", "--repo", args.repo, "--title", TITLE, "--label", LABEL,
            "--body-file", f.name,
        )  # fmt: skip
        print(out.strip())
    elif found:
        gh("issue", "comment", found, "--repo", args.repo, "--body-file", f.name)
        gh("issue", "close", found, "--repo", args.repo)
        print(f"closed #{found}")
    else:
        print("nothing serious; no issue open")
    return 0


if __name__ == "__main__":
    sys.exit(main())
