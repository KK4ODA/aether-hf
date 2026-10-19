"""Score the busy detector's candidates on the AGC corpus (ADR-0078).

Runs ``aetherd --busy-trace`` over every clip of ``tools/busy_corpus.py`` once per candidate,
and once over the clip's noise alone for the true floor, then measures what
``docs/research/busy-detection/03-candidates.md`` registers:

* **release** — after a strong signal outside the channel goes off, how long until the channel
  reads clear and stays clear for 3 s (only events with nothing in the channel nearby);
* **false busy** — busy time no signal in the channel explains (a signal's span, plus the
  hangover and a second);
* **caught** — for every transmission in the channel, the share of it called busy (from half a
  second in) and how long the first busy took;
* **floor error** — the floor against the true noise (the noise alone, after the same gain,
  through the same detector), on blocks with nothing in the channel.

    python tools/busy_eval.py CORPUS_DIR [--variants plain,normalised,...] [--only PART]
                              [--daemon PATH] [--json OUT.json] [--jobs N]
    python tools/busy_eval.py --judge OUT.json --candidate normalised

``--judge`` holds a candidate to the criteria ``03-candidates.md`` §2.2 registered, against
``plain`` on the same clips, and names every clip that fails one. Groups H, L and C2300 were added
to the corpus after the first run; they are judged as the registered groups they extend (H and L
as A, C2300 as C) and reported apart.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import math
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
BLOCK_S = 0.025
HANG_S = 0.75
CLEAR_FOR_S = 3.0
"""How long the channel must stay clear for a release to count as over."""
NEARBY_S = 75.0
"""A release is measured only when nothing in the channel starts within this of it."""


def trace(daemon: Path, wav: Path, bandwidth: int, variant: str) -> tuple[dict, float]:
    """The detector's trace of a clip, column by column, and how long it took."""
    began = time.perf_counter()
    out = subprocess.run(
        [
            str(daemon),
            "--busy-trace",
            str(wav),
            "--bandwidth",
            str(bandwidth),
            "--busy-variant",
            variant,
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    took = time.perf_counter() - began
    rows = list(csv.DictReader(io.StringIO(out)))
    cols: dict[str, np.ndarray] = {}
    for key in rows[0]:
        if key == "reason":
            cols[key] = np.array([r[key] for r in rows])
        else:
            cols[key] = np.array([float(r[key]) for r in rows])
    return cols, took


def rolling_median(x: np.ndarray, n: int) -> np.ndarray:
    pad = n // 2
    padded = np.pad(x, pad, mode="edge")
    windows = np.lib.stride_tricks.sliding_window_view(padded, n)
    return np.median(windows, axis=1)[: len(x)]


def score(meta: dict, tr: dict, truth: dict) -> dict:
    """The registered measurements of one candidate on one clip."""
    t = tr["t_s"]
    busy = tr["busy"] > 0
    n = len(t)
    explained = np.zeros(n, dtype=bool)
    channel_segments = []
    for src in meta["sources"]:
        if not src["in_channel"]:
            continue
        for start, end in src["segments"]:
            explained |= (t >= start) & (t < end + HANG_S + 1.0)
            channel_segments.append((src, start, end))
    out: dict = {"name": meta["name"], "group": meta["group"]}
    # release after each strong signal outside the channel goes off
    releases = []
    censored = 0
    for src in meta["sources"]:
        if src["in_channel"] or not src.get("windows"):
            continue
        ends = sorted(end for _, end in src["windows"])
        starts = sorted(start for start, _ in src["windows"])
        for off in ends:
            if any(abs(s - off) < NEARBY_S or s < off < e + 2 for _, s, e in channel_segments):
                continue
            nxt = min([s for s in starts if s > off] + [meta["seconds"]])
            window = (t >= off) & (t < nxt)
            idx = np.flatnonzero(window)
            need = int(CLEAR_FOR_S / BLOCK_S)
            release = None
            for k in range(len(idx) - need):
                if not busy[idx[k] : idx[k] + need].any():
                    release = t[idx[k]] - off
                    break
            if release is None:
                release = nxt - off
                censored += 1
            releases.append(max(0.0, release))
    out["releases"] = releases
    out["release_max_s"] = max(releases) if releases else math.nan
    out["release_censored"] = censored
    # false busy: what nothing in the channel explains, after the detector has settled
    settled = t > 5.0
    false = busy & ~explained & settled
    out["false_busy_s"] = float(false.sum() * BLOCK_S)
    episodes = 0
    run = 0
    for f in false:
        run = run + 1 if f else 0
        if run == int(5.0 / BLOCK_S):
            episodes += 1
    out["false_episodes_5s"] = episodes
    # each transmission in the channel: share called busy, attack
    caught = []
    for src, start, end in channel_segments:
        if end - start < 2.0:
            continue
        inside = (t >= start + 0.5) & (t < end)
        first = np.flatnonzero((t >= start) & (t < start + 5.0) & busy)
        caught.append(
            {
                "kind": src["kind"],
                "snr": src["snr_3k_db"],
                "start": start,
                "length": end - start,
                "share": float(busy[inside].mean()) if inside.any() else math.nan,
                "share_70": float(busy[inside & (t < start + 70.0)].mean())
                if inside.any()
                else math.nan,
                "attack_s": float(t[first[0]] - start) if len(first) else math.nan,
            }
        )
    out["caught"] = caught
    # the floor against the true noise, where nothing in the channel is on
    truth_db = rolling_median(truth["power_db"], 41)
    m = min(len(truth_db), n)
    quiet = (~explained[:m]) & (t[:m] > 10.0)
    err = tr["floor_db"][:m] - truth_db[:m]
    err = err[quiet & np.isfinite(err)]
    out["floor_err_median_db"] = float(np.median(np.abs(err))) if len(err) else math.nan
    out["floor_err_p95_db"] = float(np.percentile(np.abs(err), 95)) if len(err) else math.nan
    out["floor_err_over3_share"] = float((np.abs(err) > 3.0).mean()) if len(err) else math.nan
    out["busy_share"] = float(busy[settled].mean())
    return out


RELEASE_S = 3.0
SHARE_SLACK = 0.02
ATTACK_SLACK_S = 0.05
FALSE_SLACK_S = 1.0
FLOOR_SLACK_DB = 0.5
SHARE_SAME = 0.005
FLOOR_MEDIAN_DB = 1.5
FLOOR_P95_DB = 3.0
REGISTERED = {"A", "A2300", "B", "B2300", "C", "D", "E", "F", "G"}


def judge(every: list[dict], candidate: str) -> int:
    """Criteria 1, 2, 4 and 7 of ``03-candidates.md`` §2.2, clip by clip; the number failed."""
    by = {(r["name"], r["variant"]): r for r in every}
    names = sorted({r["name"] for r in every})
    failures: list[tuple[str, str, str]] = []

    def fail(criterion: str, name: str, why: str) -> None:
        failures.append((criterion, name, why))

    for name in names:
        c, p = by[(name, candidate)], by[(name, "plain")]
        group = c["group"]
        # 1: the release, for drivers outside the channel (H and L judged as A)
        if group in {"A", "A2300", "H", "L"}:
            late = [x for x in c["releases"] if x > RELEASE_S]
            if late or c["release_censored"]:
                fail(
                    "1",
                    name,
                    f"releases {', '.join(f'{x:.1f}' for x in c['releases'])} s, "
                    f"{c['release_censored']} censored",
                )
        # 2: no more missed occupancy (C2300 judged as C)
        if group in {"B", "B2300", "C", "C2300", "E", "F"}:
            key = "share_70" if group == "E" else "share"
            for cc, pc in zip(c["caught"], p["caught"], strict=True):
                sc, sp = cc[key], pc[key]
                what = f"{cc['kind']} {cc['snr']:+.0f} dB at {cc['start']:.0f} s"
                if sc < sp - SHARE_SLACK:
                    fail("2", name, f"{what}: share {sc:.2f} against plain's {sp:.2f}")
                if sp >= 0.5 and sc < 0.5:
                    fail("2", name, f"{what}: share {sc:.2f} under half, plain {sp:.2f}")
                ap_, ac = pc["attack_s"], cc["attack_s"]
                if not math.isnan(ap_) and (math.isnan(ac) or ac > ap_ + ATTACK_SLACK_S):
                    fail("2", name, f"{what}: first busy {ac:.2f} s against plain's {ap_:.2f}")
        # 4: nothing changes without an AGC
        if group in {"D", "G"} or (group == "B" and name.endswith("-off")):
            if c["false_busy_s"] > p["false_busy_s"] + FALSE_SLACK_S:
                fail(
                    "4",
                    name,
                    f"false busy {c['false_busy_s']:.1f} s against {p['false_busy_s']:.1f}",
                )
            if c["false_episodes_5s"] > p["false_episodes_5s"]:
                fail(
                    "4",
                    name,
                    f"{c['false_episodes_5s']} episodes ≥5 s against {p['false_episodes_5s']}",
                )
            if abs(c["floor_err_median_db"] - p["floor_err_median_db"]) > FLOOR_SLACK_DB:
                fail(
                    "4",
                    name,
                    f"floor error median {c['floor_err_median_db']:.2f} dB against "
                    f"{p['floor_err_median_db']:.2f}",
                )
            if abs(c["busy_share"] - p["busy_share"]) > SHARE_SAME:
                fail("4", name, f"busy share {c['busy_share']:.3f} against {p['busy_share']:.3f}")
        # 7: the floor reads the true noise with the AGC acting (H and L judged as A)
        acting = group in {"A", "A2300", "C", "D", "H", "L"} or name.startswith("F-wide")
        if acting and (
            c["floor_err_median_db"] > FLOOR_MEDIAN_DB or c["floor_err_p95_db"] > FLOOR_P95_DB
        ):
            fail(
                "7",
                name,
                f"floor error median {c['floor_err_median_db']:.2f}, "
                f"p95 {c['floor_err_p95_db']:.2f} dB",
            )
    for later in (False, True):
        chosen = [f for f in failures if (by[(f[1], candidate)]["group"] in REGISTERED) != later]
        print(
            f"\n## {candidate}: {'groups added after the first run (H, L, C2300)' if later else 'the registered groups'}"
        )
        if not chosen:
            print("every criterion met")
        for criterion in ("1", "2", "4", "7"):
            these = [f for f in chosen if f[0] == criterion]
            if these:
                print(f"criterion {criterion}: {len({f[1] for f in these})} clips")
                for _, name, why in these:
                    print(f"  {name}: {why}")
    return len(failures)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("corpus", type=Path, nargs="?")
    ap.add_argument("--judge", type=Path, help="judge the measurements in this JSON instead")
    ap.add_argument("--candidate", default="normalised")
    ap.add_argument(
        "--variants", default="plain,normalised,memory,normalised-memory,refuse,short-hold"
    )
    ap.add_argument("--only")
    ap.add_argument("--daemon", type=Path, default=ROOT / "core/target/release/aetherd")
    ap.add_argument("--json", type=Path, help="write every measurement here")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    args = ap.parse_args()
    if args.judge:
        sys.exit(1 if judge(json.loads(args.judge.read_text()), args.candidate) else 0)
    if args.corpus is None:
        ap.error("a corpus directory, or --judge")
    variants = args.variants.split(",")
    metas = sorted(args.corpus.glob("*.json"))
    metas = [m for m in metas if not args.only or args.only in m.stem]

    def one(path: Path) -> list[dict]:
        meta = json.loads(path.read_text())
        wav = path.with_suffix(".wav")
        noise = path.with_name(path.stem + ".noise.wav")
        truth, _ = trace(args.daemon, noise, meta["bandwidth"], "plain")
        results = []
        for v in variants:
            tr, took = trace(args.daemon, wav, meta["bandwidth"], v)
            r = score(meta, tr, truth)
            r["variant"] = v
            r["seconds_cpu"] = took
            results.append(r)
        return results

    every: list[dict] = []
    with ThreadPoolExecutor(args.jobs) as pool:
        for results in pool.map(one, metas):
            every.extend(results)
            print(f"{results[0]['name']}", file=sys.stderr)
    if args.json:
        args.json.write_text(json.dumps(every, indent=1))
    summarise(every, variants)


def summarise(every: list[dict], variants: list[str]) -> None:
    """A table per group and candidate."""
    groups = sorted({r["group"] for r in every})
    for g in groups:
        print(f"\n## group {g}")
        print(
            "| candidate | release max (s) | release p95 (s) | censored | false busy (s) | "
            "episodes ≥5 s | floor err median / p95 (dB) | caught share (min / mean) | "
            "attack max (s) |"
        )
        print("|---|---|---|---|---|---|---|---|---|")
        for v in variants:
            rs = [r for r in every if r["group"] == g and r["variant"] == v]
            rel = [x for r in rs for x in r["releases"]]
            caught = [c for r in rs for c in r["caught"]]
            shares = [c["share"] for c in caught if not math.isnan(c["share"])]
            attacks = [c["attack_s"] for c in caught if not math.isnan(c["attack_s"])]
            fe_med = np.nanmedian([r["floor_err_median_db"] for r in rs])
            fe_p95 = np.nanmax([r["floor_err_p95_db"] for r in rs])
            print(
                f"| {v} | {max(rel) if rel else math.nan:.1f} | "
                f"{np.percentile(rel, 95) if rel else math.nan:.1f} | "
                f"{sum(r['release_censored'] for r in rs)} | "
                f"{sum(r['false_busy_s'] for r in rs):.0f} | "
                f"{sum(r['false_episodes_5s'] for r in rs)} | "
                f"{fe_med:.1f} / {fe_p95:.1f} | "
                f"{min(shares) if shares else math.nan:.2f} / "
                f"{np.mean(shares) if shares else math.nan:.2f} | "
                f"{max(attacks) if attacks else math.nan:.2f} |"
            )


if __name__ == "__main__":
    main()
