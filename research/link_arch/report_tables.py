"""The tables of the experimental report, from a campaign's runs.

    uv run python -m research.link_arch.report_tables research/link_arch/results/main.csv.gz \
        research/link_arch/results/main_overlaps.csv.gz > research/link_arch/results/tables.md
"""

from __future__ import annotations

import math
import statistics
import sys
from collections import Counter, defaultdict
from typing import Any

from research.link_arch.analyze import boot_ci, load, paired_ratio_ci

ARMS = ["legacy", "stated", "stated6", "gs", "fc", "granted"]
NAMES = {
    "legacy": "legacy (protocol 7)",
    "stated": "ADR-0068",
    "stated6": "ADR-0068, 6-frame",
    "gs": "D: granted slots",
    "fc": "C: fixed cycle",
    "granted": "protocol 8 (model engine)",
}
PROFILES = ["ideal", "field", "field-asym", "hostile-c", "hostile"]
KINDS = ["bulk", "exchange", "chat"]
KEY = ["bw", "channel", "snr", "kind", "profile", "trial"]


def pct(x: float) -> str:
    return f"{100 * x:.1f} %"


def ci(t: tuple[float, float, float], d: int = 2) -> str:
    v, lo, hi = t
    if math.isnan(v):
        return "–"
    return f"{v:.{d}f} [{lo:.{d}f}, {hi:.{d}f}]"


def table_safety(runs: list[dict[str, str]]) -> None:
    print("### Completion and overlaps, by arm and station profile\n")
    print("Completed: every message delivered intact. Clean: both stations closed the session")
    print("with a DISC exchange. Overlaps: instants both transmitters radiated (per run, mean,")
    print("95 % bootstrap interval over runs).\n")
    print("| arm | profile | runs | completed | clean end | RF overlaps / run | runs with any |")
    print("|---|---|---:|---:|---:|---:|---:|")
    g: dict[tuple[str, str], list[dict[str, str]]] = defaultdict(list)
    for r in runs:
        g[(r["arm"], r["profile"])].append(r)
    for arm in ARMS:
        for prof in PROFILES:
            rs = g.get((arm, prof), [])
            if not rs:
                continue
            ok = sum(int(r["ok"]) for r in rs) / len(rs)
            clean = sum(int(r.get("clean_end") or 0) for r in rs) / len(rs)
            ov = [float(r.get("rf_overlaps") or 0) for r in rs]
            anyov = sum(1 for x in ov if x > 0) / len(rs)
            print(
                f"| {NAMES[arm]} | {prof} | {len(rs)} | {pct(ok)} | {pct(clean)} | "
                f"{ci(boot_ci(ov))} | {pct(anyov)} |"
            )
    print()


def table_mechanisms(overs: list[dict[str, str]], runs: list[dict[str, str]]) -> None:
    print("### Overlap mechanisms (RF overlaps per run)\n")
    n: Counter[tuple[str, str]] = Counter((r["arm"], r["profile"]) for r in runs)
    c: dict[tuple[str, str], Counter[str]] = defaultdict(Counter)
    for o in overs:
        if float(o["rf_s"]) <= 0:
            continue
        cls = o["class"]
        if cls.startswith("late-answer"):
            cls = "late answer (" + cls.split("/", 1)[1] + ")"
        elif cls.startswith("unsolicited"):
            cls = "unsolicited (" + cls.split("/", 1)[1] + ")"
        c[(o["arm"], o["profile"])][cls] += 1
    classes = sorted({k for v in c.values() for k in v})
    for prof in ("field", "hostile-c"):
        print(f"**Profile {prof}**\n")
        print("| mechanism | " + " | ".join(NAMES[a] for a in ARMS) + " |")
        print("|---|" + "---:|" * len(ARMS))
        for cls in classes:
            cells = [f"{c[(a, prof)][cls] / max(1, n[(a, prof)]):.3f}" for a in ARMS]
            if all(x == "0.000" for x in cells):
                continue
            print(f"| {cls} | " + " | ".join(cells) + " |")
        print()


def index(runs: list[dict[str, str]]) -> dict[tuple[str, ...], dict[str, dict[str, str]]]:
    idx: dict[tuple[str, ...], dict[str, dict[str, str]]] = defaultdict(dict)
    for r in runs:
        idx[tuple(r[k] for k in KEY)][r["arm"]] = r
    return idx


def table_time(runs: list[dict[str, str]], base: str = "stated") -> None:
    idx = index(runs)
    print(f"### Completion time against {NAMES[base]}, paired by seed\n")
    print("Geometric mean of the ratio of completion times over the runs both arms completed")
    print("(< 1: faster than the reference arm), 95 % bootstrap interval over pairs; and how")
    print("many of the paired runs each completed.\n")
    print("| profile | workload | " + " | ".join(NAMES[a] for a in ARMS if a != base) + " |")
    print("|---|---|" + "---|" * (len(ARMS) - 1))
    for prof in PROFILES:
        for kind in KINDS:
            cells = []
            for arm in ARMS:
                if arm == base:
                    continue
                pairs = []
                n = done_b = done_a = 0
                for k, arms in idx.items():
                    if k[4] != prof or k[3] != kind or base not in arms or arm not in arms:
                        continue
                    a, b = arms[base], arms[arm]
                    n += 1
                    done_b += int(a["ok"])
                    done_a += int(b["ok"])
                    if int(a["ok"]) and int(b["ok"]):
                        pairs.append((float(a["done_s"]), float(b["done_s"])))
                cells.append(f"×{ci(paired_ratio_ci(pairs), 3)}<br>{done_b} vs {done_a} of {n}")
            print(f"| {prof} | {kind} | " + " | ".join(cells) + " |")
    print()


def table_by_air(runs: list[dict[str, str]], prof: str = "field", base: str = "stated") -> None:
    idx = index(runs)
    print(f"### Completion time against {NAMES[base]} by air and channel ({prof} profile)\n")
    print("| air | channel | workload | " + " | ".join(NAMES[a] for a in ARMS if a != base) + " |")
    print("|---|---|---|" + "---|" * (len(ARMS) - 1))
    for bw in ("2300", "500"):
        for ch in ("awgn", "good", "moderate", "poor"):
            for kind in KINDS:
                cells = []
                for arm in ARMS:
                    if arm == base:
                        continue
                    pairs = []
                    for k, arms in idx.items():
                        if (k[0], k[1], k[3], k[4]) != (bw, ch, kind, prof):
                            continue
                        if (
                            base in arms
                            and arm in arms
                            and int(arms[base]["ok"])
                            and int(arms[arm]["ok"])
                        ):
                            pairs.append((float(arms[base]["done_s"]), float(arms[arm]["done_s"])))
                    v, _lo, _hi = paired_ratio_ci(pairs)
                    cells.append(f"×{v:.2f}" if not math.isnan(v) else "–")
                print(f"| {bw} | {ch} | {kind} | " + " | ".join(cells) + " |")
    print()


def table_goodput(runs: list[dict[str, str]], prof: str = "field") -> None:
    print(f"### Bulk goodput (bit/s, median over trials and channels), {prof} profile\n")
    print("| air | SNR dB | " + " | ".join(NAMES[a] for a in ARMS) + " |")
    print("|---|---:|" + "---:|" * len(ARMS))
    g: dict[tuple[str, str, str], list[float]] = defaultdict(list)
    for r in runs:
        if r["profile"] == prof and r["kind"] == "bulk":
            g[(r["arm"], r["bw"], r["snr"])].append(float(r["goodput_bps"]) if int(r["ok"]) else 0)
    for bw in ("2300", "500"):
        for snr in ("-6.0", "-3.0", "0.0", "3.0", "6.0", "9.0", "12.0"):
            cells = [
                f"{statistics.median(g[(a, bw, snr)]):.0f}" if g[(a, bw, snr)] else "–"
                for a in ARMS
            ]
            print(f"| {bw} | {snr} | " + " | ".join(cells) + " |")
    print()


def table_chat(runs: list[dict[str, str]]) -> None:
    print("### Chat: a line's latency and the air it costs\n")
    print("Median over sessions of each session's median line latency; 90th percentile")
    print("over sessions of each session's slowest line; transmitters' duty cycle (keyed")
    print("seconds over session seconds, both stations).\n")
    print(
        "| profile | arm | sessions completed | median line latency s | p90 of slowest s | duty |"
    )
    print("|---|---|---:|---:|---:|---:|")
    for prof in ("field", "hostile-c"):
        for arm in ARMS:
            rs = [
                r for r in runs if r["arm"] == arm and r["profile"] == prof and r["kind"] == "chat"
            ]
            ok = [r for r in rs if int(r["ok"])]
            lat = [float(r["lat_med_s"]) for r in ok if r["lat_med_s"]]
            worst = sorted(float(r["lat_max_s"]) for r in ok if r["lat_max_s"])
            duty = [(float(r["air_a_s"]) + float(r["air_b_s"])) / float(r["done_s"]) for r in ok]
            if not lat:
                continue
            print(
                f"| {prof} | {NAMES[arm]} | {len(ok)}/{len(rs)} | {statistics.median(lat):.1f} | "
                f"{worst[int(0.9 * len(worst))]:.1f} | {statistics.fmean(duty):.3f} |"
            )
    print()


def table_air(runs: list[dict[str, str]], prof: str = "field") -> None:
    print(f"### Air per delivered kilobyte (keyed seconds, both stations), {prof} profile\n")
    print("| workload | " + " | ".join(NAMES[a] for a in ARMS) + " |")
    print("|---|" + "---:|" * len(ARMS))
    for kind in KINDS:
        cells = []
        for arm in ARMS:
            rs = [
                r
                for r in runs
                if r["arm"] == arm and r["profile"] == prof and r["kind"] == kind and int(r["ok"])
            ]
            per_kb: list[float] = []
            for r in rs:
                kb = float(r["goodput_bps"]) * float(r["done_s"]) / 8 / 1000
                if kb > 0:
                    per_kb.append((float(r["air_a_s"]) + float(r["air_b_s"])) / kb)
            cells.append(f"{statistics.median(per_kb):.1f}" if per_kb else "–")
        print(f"| {kind} | " + " | ".join(cells) + " |")
    print()


def table_ends(runs: list[dict[str, str]]) -> None:
    print("### How sessions that did not complete ended (all profiles)\n")
    print("| arm | not completed | most common ends (caller, called) |")
    print("|---|---:|---|")
    for arm in ARMS:
        rs = [r for r in runs if r["arm"] == arm and not int(r["ok"])]
        c = Counter((r["end_a"], r["end_b"]) for r in rs)
        text = "; ".join(f"{a} / {b}: {n}" for (a, b), n in c.most_common(4))
        print(f"| {NAMES[arm]} | {len(rs)} | {text} |")
    print()


def main(argv: list[str]) -> int:
    runs = load(argv[1])
    overs: list[dict[str, Any]] = load(argv[2]) if len(argv) > 2 else []
    table_safety(runs)
    if overs:
        table_mechanisms(overs, runs)
    table_time(runs)
    table_time(runs, base="legacy")
    table_by_air(runs)
    table_goodput(runs)
    table_chat(runs)
    table_air(runs)
    table_ends(runs)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
