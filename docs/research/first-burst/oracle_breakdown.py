"""Where the controller's time goes, against the best fixed rung in hindsight.

Reads a directory of ``tools/rung_oracle.py`` runs (``<scenario>.summary.csv`` beside
``<scenario>/<name or name-kN>/auto/{a,b}/recordings/*.json``) and splits the controller's own
data air by whether each frame went below, at or above the draw's best fixed rung, and how much
went on copies of blocks already sent (02-oracle-breakdown.md).

    python -I docs/research/first-burst/oracle_breakdown.py ORACLE_DIR
"""

import csv
import json
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(sys.argv[1])
FLOOR_S, ORD_S = 5.36, 1.054

CLASS = {
    "strong": [
        "quick-awgn-2300",
        "40m-good-throughput-2300",
        "80m-steady-mail-2300",
        "80m-steady-mail-500",
        "80m-strong-moderate-500",
        "80m-start-low-reading-500",
        "80m-start-high-reading-500",
    ],
    "fading": [
        "20m-dx-500",
        "20m-dx-moderate-2300",
        "80m-qsb-dropout-500",
        "80m-poor-500-pactor-signalink",
        "80m-lopsided-acks-500",
        "80m-slow-machine-500",
    ],
    "fast fading": ["15m-equatorial-flutter-2300", "80m-flutter-500", "20m-polar-path-2300"],
    "NVIS echo": [
        "80m-agc-carrier-outside-500",
        "80m-nvis-disturbed-500",
        "80m-nvis-evening-500",
        "80m-trimode-client-vanishes-500",
    ],
    "weak": ["80m-weak-500", "80m-weak-both-ways-ladder-500", "80m-weak-wide-test-2300"],
    "QRM": [
        "40m-good-2300-rtty-crashes",
        "40m-moderate-2300-vara-qrm",
        "80m-thunderstorm-agc-2300",
    ],
}
CLASS_OF = {s: c for c, ss in CLASS.items() for s in ss}


def data_frames(run: Path):
    out = []
    for p in run.glob("*/recordings/*.json"):
        try:
            d = json.loads(p.read_text())
        except ValueError:
            continue
        if not isinstance(d, dict):
            continue
        for e in d.get("sent") or []:
            if e.get("kind") == "data" and "seq" in e and not e.get("measure"):
                out.append((p.parts[-3], e))
    return out


rows = []
for summ in sorted(ROOT.glob("*.summary.csv")):
    sc = summ.name.removesuffix(".summary.csv")
    for r in csv.DictReader(summ.open()):
        seed = int(r["seed"])
        k = (seed - int(sorted({int(x["seed"]) for x in csv.DictReader(summ.open())})[0])) // 1000
        inner = r["scenario"]
        run = ROOT / sc / (inner if k == 0 else f"{inner}-k{k}") / "auto"
        best = int(r["best_rung"])
        frames = data_frames(run)
        air = Counter()
        again = 0.0
        seen = set()
        first = {}
        for who, e in sorted(frames, key=lambda x: x[1]["t_s"]):
            s = FLOOR_S if e.get("floor") else ORD_S
            m = e["mode"]
            side = "below" if m < best else "at" if m == best else "above"
            air[side] += s
            key = (who, e["seq"], e.get("piece"))
            if key in seen:
                again += s
            seen.add(key)
            first.setdefault(who, m)
        res = (
            json.loads((run / "result.json").read_text()) if (run / "result.json").exists() else {}
        )
        rows.append(
            dict(
                sc=sc,
                cls=CLASS_OF.get(sc, "?"),
                seed=seed,
                best=best,
                regret=float(r["regret"]),
                auto=float(r["auto_metric"]),
                bestm=float(r["best_metric"]),
                air_s=res.get("air_s"),
                below=air["below"],
                at=air["at"],
                above=air["above"],
                again=again,
                first=" ".join(f"{w}:{m}" for w, m in sorted(first.items())),
                rungs=" ".join(f"{m}" for m in sorted({e["mode"] for _, e in frames})),
            )
        )

print(
    "| class | scenario | seed | best | ctrl | best m | regret"
    " | data air below/at/above best (s) | sent again (s) | first rung | rungs used |"
)
print("|---|---|---|---|---|---|---|---|---|---|---|")
for x in sorted(rows, key=lambda x: (x["cls"], x["sc"], x["seed"])):
    print(
        f"| {x['cls']} | {x['sc']} | {x['seed']} | {x['best']} | {x['auto']:.0f} | "
        f"{x['bestm']:.0f} | {x['regret']:+.0%} | "
        f"{x['below']:.0f}/{x['at']:.0f}/{x['above']:.0f} | {x['again']:.0f} | "
        f"{x['first']} | {x['rungs']} |"
    )
print()
by = defaultdict(list)
for x in rows:
    by[x["cls"]].append(x)
print("| class | runs | median regret | data air below best | at | above | sent again |")
print("|---|---|---|---|---|---|---|")
tot = Counter()
for c, xs in sorted(by.items()):
    b = sum(x["below"] for x in xs)
    a = sum(x["at"] for x in xs)
    ab = sum(x["above"] for x in xs)
    t = b + a + ab or 1
    tot.update(below=b, at=a, above=ab, again=sum(x["again"] for x in xs))
    print(
        f"| {c} | {len(xs)} | {statistics.median(x['regret'] for x in xs):+.0%} | {b / t:.0%} | "
        f"{a / t:.0%} | {ab / t:.0%} | {sum(x['again'] for x in xs) / t:.0%} |"
    )
t = tot["below"] + tot["at"] + tot["above"]
print(
    f"| all | {len(rows)} | {statistics.median(x['regret'] for x in rows):+.0%} | "
    f"{tot['below'] / t:.0%} | {tot['at'] / t:.0%} | {tot['above'] / t:.0%} | "
    f"{tot['again'] / t:.0%} |"
)
