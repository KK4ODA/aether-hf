"""Run a grid of arms × conditions × trials on the adversarial simulator.

    uv run python -m research.link_arch.campaign --arms legacy,stated,stated6 \
        --bandwidth 500,2300 --channels good,moderate,poor --snr -6,0,6,12 \
        --kinds bulk,exchange,chat --profiles ideal,field,hostile --trials 8 \
        --out research/link_arch/results/classify.csv --overlaps-out ...overlaps.csv

Every arm runs the same trials (seeds), channels and scripts. One row per run; with
``--overlaps-out`` one row per overlap with both transmissions' attribution.
"""

from __future__ import annotations

import argparse
import csv
import itertools
import time
from multiprocessing import Pool
from pathlib import Path
from typing import Any

from research.link_arch.classify import kind_of
from research.link_arch.setups import builder
from research.link_arch.workload import run

PROFILE_ASYM = {"field-asym": ("field", 6.0), "hostile-asym": ("hostile", 6.0)}


def one(job: tuple[Any, ...]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    arm, bw, ch, snr, kind, prof, trial, nbytes, cs = job
    base, asym = PROFILE_ASYM.get(prof, (prof, 0.0))
    make = builder(
        arm, bandwidth=bw, channel=ch, snr_db=snr, asym_db=asym, profile=base, carrier_sense=cs
    )
    t = time.perf_counter()
    try:
        row = run(make, kind, trial, nbytes=nbytes)
    except Exception as exc:  # a crash is a result too: record it
        row = {
            "kind": kind,
            "trial": trial,
            "ok": 0,
            "error": repr(exc)[:200],
            "_overlaps": [],
            "_classes": [],
        }
    over, classes = row.pop("_overlaps"), row.pop("_classes")
    head = {"arm": arm, "bw": bw, "channel": ch, "snr": snr, "profile": prof, "cs": int(cs)}
    row = {**head, **row, "wall_s": round(time.perf_counter() - t, 2)}
    orows = []
    for (x, y, k, r), c in zip(over, classes, strict=False):
        kx, ky = kind_of(x), kind_of(y)
        orows.append(
            {
                **head,
                "kind": kind,
                "trial": trial,
                "class": c,
                "keyed_s": round(k, 3),
                "rf_s": round(r, 3),
                "first_who": x.who,
                "first_t": round(x.rad[0], 2),
                "first_kind": "/".join(kx),
                "first_frames": "+".join(x.kinds),
                "first_trig": x.trigger,
                "first_hold": round(x.hold_s, 2),
                "first_lead": round(x.lead_extra_s, 2),
                "second_who": y.who,
                "second_t": round(y.rad[0], 2),
                "second_kind": "/".join(ky),
                "second_frames": "+".join(y.kinds),
                "second_trig": y.trigger,
                "second_hold": round(y.hold_s, 2),
                "second_lead": round(y.lead_extra_s, 2),
                "first_role": x.belief_self.get("role", x.belief_self.get("mode")),
                "second_role": y.belief_self.get("role", y.belief_self.get("mode")),
                "first_why": x.reason[-120:],
                "second_why": y.reason[-120:],
            }
        )
    return row, orows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arms", default="legacy,stated")
    ap.add_argument("--bandwidth", default="500,2300")
    ap.add_argument("--channels", default="good,moderate,poor")
    ap.add_argument("--snr", default="-6,0,6,12")
    ap.add_argument("--kinds", default="bulk,exchange,chat")
    ap.add_argument("--profiles", default="field")
    ap.add_argument("--trials", type=int, default=8)
    ap.add_argument("--first-trial", type=int, default=0)
    ap.add_argument("--bytes", type=int, default=4000)
    ap.add_argument("--carrier-sense", default="0")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", required=True)
    ap.add_argument("--overlaps-out", default="")
    args = ap.parse_args()
    jobs = list(
        itertools.product(
            args.arms.split(","),
            [int(b) for b in args.bandwidth.split(",")],
            args.channels.split(","),
            [float(s) for s in args.snr.split(",")],
            args.kinds.split(","),
            args.profiles.split(","),
            range(args.first_trial, args.first_trial + args.trials),
            [args.bytes],
            [bool(int(c)) for c in args.carrier_sense.split(",")],
        )
    )
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    orows: list[dict[str, Any]] = []
    t0 = time.perf_counter()
    with Pool(args.jobs) as pool:
        for i, (row, ov) in enumerate(pool.imap_unordered(one, jobs, chunksize=4)):
            rows.append(row)
            orows.extend(ov)
            if (i + 1) % 200 == 0:
                print(f"{i + 1}/{len(jobs)} {time.perf_counter() - t0:.0f}s", flush=True)
    keys: list[str] = []
    for r in rows:
        keys.extend(k for k in r if k not in keys)
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)
    if args.overlaps_out and orows:
        with open(args.overlaps_out, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=list(orows[0]))
            w.writeheader()
            w.writerows(orows)
    print(f"{len(rows)} runs, {len(orows)} overlaps, {time.perf_counter() - t0:.0f}s")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
