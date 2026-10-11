"""Open, update or close the GitHub issue that tracks a failing nightly run.

    python tools/nightly_report.py --runs runs/ --results "full=success scenarios=failure …"
                                   --run-url URL --sha SHA --repo OWNER/NAME [--dry-run]

The email GitHub sends for a failed scheduled run says only that it failed. This reads what
the nightly's jobs left (``--runs``: the run's artifacts, one folder each — the scenario
variants' ``results.csv``, the trend table, ``soak.json``, the mixed pair's ``results.csv``)
and writes what failed and why into an issue labelled ``nightly``: a new one when none is
open, a comment on the open one otherwise, so a broken week is one thread. A night that
passes says so on the open issue and closes it.

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


def failed_rows(path: Path) -> list[str]:
    with path.open(newline="", encoding="utf-8") as f:
        return [
            f"- `{r['scenario']}`: {r.get('why') or 'failed'}"
            for r in csv.DictReader(f)
            if r.get("pass") != "True"
        ]


def body(runs: Path, results: dict[str, str], run_url: str, sha: str, today: str) -> str:
    failed = [job for job, result in results.items() if result == "failure"]
    lines = [
        f"**{today}**, `{sha[:7]}`: {', '.join(failed) or 'nothing'} failed — [the run]({run_url}).",
        "",
    ]
    if "full" in failed:
        lines += ["**Tests:** the full pytest set failed; the run's log has the test.", ""]
    scenario_lines: list[str] = []
    for csv_path in sorted(runs.glob("scenarios-*/results.csv")):
        scenario_lines += failed_rows(csv_path)
    if scenario_lines:
        lines += ["**Scenarios** (each failing variant):", *scenario_lines, ""]
    elif "scenarios" in failed:
        lines += ["**Scenarios:** the job failed before writing results.", ""]
    trend = runs / "trend" / "trend.md"
    if "trend" in failed and trend.exists():
        head = trend.read_text(encoding="utf-8").split("<details>")[0].strip()
        lines += ["**Speed against the baseline:**", "", head.replace("## ", "### "), ""]
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
    note = mixed / "note.md"
    if note.exists():
        lines += ["**Against the last release:** " + note.read_text(encoding="utf-8").strip(), ""]
    mixed_lines: list[str] = []
    for csv_path in sorted(mixed.glob("*/results.csv")):
        mixed_lines += [f"{line} ({csv_path.parent.name})" for line in failed_rows(csv_path)]
    if mixed_lines:
        lines += ["**This build with the last release:**", *mixed_lines, ""]
    lines.append("Closed by the first night that passes.")
    return "\n".join(lines)


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
    failing = any(r == "failure" for r in results.values())
    text = (
        body(args.runs, results, args.run_url, args.sha, today)
        if failing
        else f"Passed on {today} at `{args.sha[:7]}` — [the run]({args.run_url}). Closing."
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
        print("passed; no issue open")
    return 0


if __name__ == "__main__":
    sys.exit(main())
