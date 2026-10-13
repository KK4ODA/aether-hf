"""Step 3's judge: protocol 8 against protocol 7 on the scenario harness, by the criteria
registered in ``docs/research/link-architecture/10-step-3.md`` §1 before either arm ran.

    uv run python research/link_arch/step3.py collect <runs-dir> <out.csv>
    uv run python research/link_arch/step3.py judge <protocol-7.csv> <protocol-8.csv>

``collect`` reads every run of a ``session_matrix.py --out`` directory (its ``result.json``,
``channel.json`` and ``scenario.toml``) into one line a run: what the criteria need, and
nothing that must be kept with the audio. ``judge`` pairs the two arms' runs by scenario and
seed and prints the tables and the verdict of each criterion; it exits 1 when any fails.

The rules here are the registration's, word for word in code; a change to them after the runs
is a change to the registration and is said so in the report.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import re
import statistics
import sys
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import Any

# ── the registration's constants (10-step-3.md §1)

INITIATIONS = {"connect", "probe", "beacon", "datagram", "drive", "test", "identifier"}
"""Transmissions a station starts on its own, outside any exchange with the other: a call's
first try, a probe's, a beacon, a datagram, drive bursts, a keying test, a lone identifier.
Carrier sense orders them, not the link protocol."""

BUILT_IN_COVER_S = 0.65
"""What protocol 8 leaves after the end of the other station's frames whatever it declared:
``D + KEY`` (0.30 + 0.35 s), the guard not counted."""

LINE_TAIL_S = 0.25
"""The longest keyed tail the daemon holds after its frames (ADR-0038's bound)."""

CLASS_LIMIT = 1.15
POOLED_LIMIT = 1.10
RATE_LIMIT = 0.85
MIN_RATED_RUNS = 3

COLLISION_FAILURE = re.compile(r"^\d+ collisions$")


def scenario_class(scenario: dict[str, Any]) -> str:
    """The five classes, disjoint, by the first rule that matches."""
    tags = set(scenario.get("tags", []))
    name = str(scenario.get("name", ""))
    if tags & {"gateway", "host"}:
        return "gateway"
    if tags & {"robustness", "machine", "vox"}:
        return "robustness"
    if name.startswith(("20m", "15m")):
        return "band"
    if int(scenario.get("bandwidth", 2300)) == 500:
        return "500 Hz"
    return "2300 Hz"


def completed(failures: str) -> bool:
    """A run completes when nothing failed but the old keyed-overlap count."""
    return all(COLLISION_FAILURE.match(f.strip()) for f in failures.split(";") if f.strip())


def s4_holds(scenario: dict[str, Any], who: str) -> bool:
    """Whether a station's radio hears again within what the protocol leaves it: its deafness
    after its frames — a VOX hold and the receiver's recovery, or the daemon's keyed tail and
    the recovery — no longer than its declared turnaround or the built-in cover."""
    station = scenario.get("stations", {}).get(who, {})
    radio = station.get("radio", {})
    hold = float(radio.get("vox_hold_ms", 0)) / 1000
    recovery = float(radio.get("rx_recovery_ms", 0)) / 1000
    deafness = (hold if hold > 0 else LINE_TAIL_S) + recovery
    declared = float(station.get("turnaround_ms", 0)) / 1000
    return deafness <= max(declared, BUILT_IN_COVER_S) + 1e-9


def main_peer(who: str) -> str:
    """As the channel server's: the gateway ``b`` for every other station, ``a`` for ``b``."""
    return "a" if who == "b" else "b"


DEFAULT_CALLSIGNS = {"a": "W4ODA", "b": "KK4XYZ", "c": "N4CCC", "d": "N4DDD", "e": "N4EEE"}
"""``session_matrix.py``'s, for a station whose scenario names no callsign."""


def station_keys(scenario: dict[str, Any], names: list[str]) -> dict[str, str]:
    """The scenario's key (``a``, ``b``, …) of each station the channel's account names: it
    names them by the callsign each daemon gave it."""
    stations = scenario.get("stations", {})
    keys = set(DEFAULT_CALLSIGNS) | set(stations)
    by_call = {
        str(stations.get(k, {}).get("callsign", DEFAULT_CALLSIGNS.get(k, k))): k for k in keys
    }
    return {name: by_call.get(name, name) for name in names}


# ── one run


def is_initiation(tx: dict[str, Any] | None) -> bool:
    return tx is not None and str(tx.get("cause", "")) in INITIATIONS


def on_air_at(transmissions: list[dict[str, Any]], t: float) -> dict[str, Any] | None:
    """The station's transmission that began last at or before ``t``."""
    best = None
    for tx in transmissions:
        if float(tx["air_s"]) <= t + 1e-6:
            best = tx
        else:
            break
    return best


def overlaps(report: dict[str, Any]) -> tuple[int, int, float]:
    """Radiated overlaps: (counted, both initiations, counted seconds). An overlap is put down
    to the transmission each station had on the air at its start, as ``air_account.py`` does."""
    stations = report.get("stations", {})
    counted = initiations = 0
    seconds = 0.0
    for start, end in report.get("rf_overlaps", []):
        on = [on_air_at(st.get("transmissions", []), float(start)) for st in stations.values()]
        present = sorted((tx for tx in on if tx is not None), key=lambda tx: float(tx["air_s"]))
        if len(present) >= 2 and is_initiation(present[-1]) and is_initiation(present[-2]):
            initiations += 1
            continue
        counted += 1
        seconds += float(end) - float(start)
    return counted, initiations, seconds


def _intersect(a: list[list[float]], b: list[list[float]]) -> list[tuple[float, float]]:
    out = []
    for a0, a1 in a:
        for b0, b1 in b:
            lo, hi = max(a0, b0), min(a1, b1)
            if hi > lo:
                out.append((lo, hi))
    return out


def deaf_hits(report: dict[str, Any], scenario: dict[str, Any]) -> tuple[float, float, str]:
    """Frames that reached a deaf receiver: (counted seconds, reported seconds, detail). The
    seconds are the channel server's (its own transmission aside, which is an overlap). A
    station's hits count when its radio keeps S4 and not every transmission found arriving in
    its deafness is an initiation; otherwise they are reported."""
    counted = reported = 0.0
    detail = []
    stations = report.get("stations", {})
    key = station_keys(scenario, list(stations))
    name_of = {k: name for name, k in key.items()}
    for name, st in stations.items():
        hits = sum(float(v) for k, v in st.get("deaf_hits_s", {}).items() if k != "transmitting")
        if hits <= 0.0:
            continue
        who = key[name]
        peer = main_peer(who)
        peer_st = stations.get(name_of.get(peer, peer), {})
        radio = scenario.get("stations", {}).get(who, {}).get("radio", {})
        after = (float(radio.get("vox_hold_ms", 0)) + float(radio.get("rx_recovery_ms", 0))) / 1000
        deaf = [[float(k0), float(k1) + after] for k0, k1 in st.get("keyed", [])]
        path = scenario.get("path", {})
        delay = float({**path, **path.get(f"{peer}_to_{who}", {})}.get("delay_ms", 0.0)) / 1000
        arriving = [
            [float(r0) + delay, float(r1) + delay] for r0, r1 in peer_st.get("radiated", [])
        ]
        found = _intersect(deaf, arriving)
        txs = peer_st.get("transmissions", [])
        causes = {str((on_air_at(txs, lo - delay) or {}).get("cause", "?")) for lo, _ in found}
        by_initiation = bool(causes) and all(c in INITIATIONS for c in causes)
        keeps = s4_holds(scenario, who)
        if keeps and not by_initiation:
            counted += hits
        else:
            reported += hits
        why = (
            "counted"
            if keeps and not by_initiation
            else ("S4 broken" if not keeps else "initiation")
        )
        detail.append(f"{name} {hits:.3f} s {why} ({'+'.join(sorted(causes)) or 'unplaced'})")
    return counted, reported, "; ".join(detail)


def collect_run(run: Path) -> dict[str, Any] | None:
    try:
        result = json.loads((run / "result.json").read_text(encoding="utf-8"))
        report = json.loads((run / "channel.json").read_text(encoding="utf-8"))
        scenario = tomllib.loads((run / "scenario.toml").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    rf, rf_initiations, rf_s = overlaps(report)
    deaf_counted, deaf_reported, deaf_detail = deaf_hits(report, scenario)
    return {
        "base": result.get("base"),
        "seed": result.get("seed"),
        "class": scenario_class(scenario),
        "completed": completed(str(result.get("failures", result.get("why", "")))),
        "failures": result.get("failures", ""),
        "air_s": result.get("air_s"),
        "rf_counted": rf,
        "rf_counted_s": round(rf_s, 3),
        "rf_initiations": rf_initiations,
        "deaf_counted_s": round(deaf_counted, 3),
        "deaf_reported_s": round(deaf_reported, 3),
        "deaf_detail": deaf_detail,
        "vox_s": result.get("vox_s"),
        "guard_s": result.get("guard_s"),
        "test": result.get("test", ""),
        "message_bps": result.get("message_bps"),
        "file_bps": result.get("file_bps"),
        "wall_s": result.get("wall_s"),
    }


def collect(runs: Path, out: Path) -> int:
    rows = []
    broken = []
    for run in sorted(p for p in runs.iterdir() if p.is_dir()):
        row = collect_run(run)
        if row is None:
            broken.append(run.name)
        else:
            rows.append(row)
    if not rows:
        print(f"no runs in {runs}", file=sys.stderr)
        return 2
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    print(f"{len(rows)} runs → {out}" + (f"; unreadable: {', '.join(broken)}" if broken else ""))
    return 0


# ── the verdict


def read(path: Path) -> dict[tuple[str, int], dict[str, str]]:
    with path.open(encoding="utf-8") as f:
        return {(r["base"], int(r["seed"])): r for r in csv.DictReader(f)}


def _num(text: str | None) -> float | None:
    try:
        value = float(text) if text not in (None, "", "None") else None
    except ValueError:
        return None
    return value if value is not None and math.isfinite(value) else None


def gmean(xs: list[float]) -> float:
    return math.exp(statistics.fmean(math.log(x) for x in xs)) if xs else float("nan")


def judge(p7_path: Path, p8_path: Path) -> int:
    p7, p8 = read(p7_path), read(p8_path)
    paired = sorted(set(p7) & set(p8))
    unpaired = sorted(set(p7) ^ set(p8))
    by_scenario: dict[str, list[tuple[dict[str, str], dict[str, str]]]] = defaultdict(list)
    for key in paired:
        by_scenario[key[0]].append((p7[key], p8[key]))

    lines = [
        "| scenario | class | runs | completed 7 / 8 | air 7 | air 8 | ×"
        " | radiated 7 / 8 | deaf 7 / 8 (s) | message 7 / 8 | file 7 / 8 |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    ratios: dict[str, list[float]] = defaultdict(list)
    all_ratios: list[float] = []
    rate_ratios: dict[str, list[float]] = {"message_bps": [], "file_bps": []}
    short = []
    radiated8 = 0
    deaf8 = 0.0
    for name in sorted(by_scenario):
        pairs = by_scenario[name]
        cls = pairs[0][1]["class"]
        done7 = sum(a["completed"] == "True" for a, _ in pairs)
        done8 = sum(b["completed"] == "True" for _, b in pairs)
        if done8 < done7:
            short.append(f"{name} {done8} < {done7}")
        air7 = statistics.median(float(a["air_s"]) for a, _ in pairs)
        air8 = statistics.median(float(b["air_s"]) for _, b in pairs)
        r = air8 / air7
        ratios[cls].append(r)
        all_ratios.append(r)
        rf7 = sum(int(a["rf_counted"]) for a, _ in pairs)
        rf8 = sum(int(b["rf_counted"]) for _, b in pairs)
        radiated8 += rf8
        d7 = sum(float(a["deaf_counted_s"]) for a, _ in pairs)
        d8 = sum(float(b["deaf_counted_s"]) for _, b in pairs)
        deaf8 += d8
        rates = []
        for key in ("message_bps", "file_bps"):
            v7 = [x for a, _ in pairs if (x := _num(a[key])) is not None]
            v8 = [x for _, b in pairs if (x := _num(b[key])) is not None]
            if len(v7) >= MIN_RATED_RUNS and len(v8) >= MIN_RATED_RUNS:
                m7, m8 = statistics.median(v7), statistics.median(v8)
                rate_ratios[key].append(m8 / m7)
                rates.append(f"{m7:.0f} / {m8:.0f}")
            elif v7 or v8:
                rates.append(f"({len(v7)}, {len(v8)} runs)")
            else:
                rates.append("")
        lines.append(
            f"| `{name}` | {cls} | {len(pairs)} | {done7} / {done8} | {air7:.0f} | {air8:.0f}"
            f" | {r:.2f} | {rf7} / {rf8} | {d7:.2f} / {d8:.2f} | {rates[0]} | {rates[1]} |"
        )
    print("\n".join(lines))
    print()
    print("| class | scenarios | air time × (geometric mean) |")
    print("|---|---:|---:|")
    class_fail = []
    for cls in sorted(ratios):
        g = gmean(ratios[cls])
        if g > CLASS_LIMIT:
            class_fail.append(f"{cls} ×{g:.3f}")
        print(f"| {cls} | {len(ratios[cls])} | {g:.3f} |")
    pooled = gmean(all_ratios)
    print(f"| pooled | {len(all_ratios)} | {pooled:.3f} |")
    print()
    rate = {k: gmean(v) for k, v in rate_ratios.items()}
    verdicts = [
        ("C1a radiated overlaps (protocol 8)", radiated8 == 0, f"{radiated8}"),
        ("C1b deaf hits counted (protocol 8)", deaf8 == 0.0, f"{deaf8:.3f} s"),
        ("C2 completion, scenario by scenario", not short, "; ".join(short) or "none short"),
        ("C3 air time, every class ≤ ×1.15", not class_fail, "; ".join(class_fail) or "all within"),
        ("C3 air time, pooled ≤ ×1.10", pooled <= POOLED_LIMIT, f"×{pooled:.3f}"),
    ]
    for key, label in (("message_bps", "message"), ("file_bps", "file")):
        n = len(rate_ratios[key])
        verdicts.append(
            (
                f"C4 Test {label} rate ≥ ×0.85 pooled",
                n > 0 and rate[key] >= RATE_LIMIT,
                f"×{rate[key]:.3f} over {n}",
            )
        )
    print("| criterion | holds | value |")
    print("|---|---|---|")
    for label, ok, value in verdicts:
        print(f"| {label} | {'yes' if ok else '**no**'} | {value} |")
    if unpaired:
        print(f"\nunpaired runs (left out): {', '.join(f'{n} s{s}' for n, s in unpaired)}")
    return 0 if all(ok for _, ok, _ in verdicts) else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="what", required=True)
    c = sub.add_parser("collect")
    c.add_argument("runs", type=Path)
    c.add_argument("out", type=Path)
    j = sub.add_parser("judge")
    j.add_argument("p7", type=Path)
    j.add_argument("p8", type=Path)
    args = ap.parse_args()
    if args.what == "collect":
        return collect(args.runs, args.out)
    return judge(args.p7, args.p8)


if __name__ == "__main__":
    raise SystemExit(main())
