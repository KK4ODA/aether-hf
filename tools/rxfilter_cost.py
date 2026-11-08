"""What a receive cut costs a session, on the scenario harness (the receive-filter review,
Gate 0 items 6 and 7).

    python tools/rxfilter_cost.py OUT_DIR [--seeds 3] [--jobs 4] [--only NAME-PART]
                                          [--daemon PATH] [--summary-only]

Writes one Test scenario per configuration into ``OUT_DIR/scenarios``, runs them through
``tools/session_matrix.py`` (``--seeds`` draws each, paired by seed across configurations) and
sets each one's Test rates beside the uncut run's on the same draw. The called station receives
the Test's message and file; its radio is the one cut (``rx_filter_hz``, an eighth-order
band-pass placed so the response is the stated depth down at the signal's edge: 360 or 2640 Hz
at 2300 Hz, 1260 or 1740 Hz at 500 Hz), with its audio chain's floor ``rx_floor_db`` under the
band's noise (15 dB as the mobile's on 2026-11-08, or 50 as the FTDX10's), or driven hot
(``rx_gain_db``, clipping at full scale).
"""

from __future__ import annotations

import argparse
import csv
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EDGES_HZ = {2300: (360.0, 2640.0), 500: (1260.0, 1740.0)}
PATHS = {2300: ("good", 15.0), 500: ("good", 12.0)}
"""Each bandwidth's path: a strong Good one, so the Test's ladder climbs to where the outer
carriers count."""
SEEDS = {2300: 601, 500: 602}
DEPTHS_DB = (3.0, 6.0, 10.0, 20.0)
FLOORS_DB = {"shallow": 15.0, "deep": 50.0}
HOT_DB = (6.0, 10.0, 14.0)
"""How far the codec is driven over the scenario's own level: the signal plays at −15 dBFS
RMS (``tx_level`` 0.25), so +8 dB is the mobile's −7 dBFS on 2026-11-08."""
ORDER = 8
WIDE_HZ = (30.0, 3600.0)
"""The side of the passband left alone: a low-pass alone at the bottom, well clear at the top."""


def corner_for(edge_hz: float, depth_db: float, side: str) -> float:
    """The corner of an eighth-order Butterworth that is ``depth_db`` down at ``edge_hz``."""
    ratio = (10.0 ** (depth_db / 10.0) - 1.0) ** (1.0 / (2 * ORDER))
    return edge_hz / ratio if side == "high" else edge_hz * ratio


def configurations() -> list[dict]:
    """Every configuration: uncut, each depth at each edge over the shallow floor, the deepest
    over the deep floor, and the hot codec."""
    out = []
    for bandwidth, (low_edge, high_edge) in EDGES_HZ.items():
        for floor, floor_db in FLOORS_DB.items():
            depths = DEPTHS_DB if floor == "shallow" else (20.0,)
            out.append(
                {"name": f"{bandwidth}-uncut-{floor}", "bandwidth": bandwidth, "floor": floor_db}
            )
            for depth in depths:
                for side, edge in (("high", high_edge), ("low", low_edge)):
                    corner = corner_for(edge, depth, side)
                    band = (WIDE_HZ[0], corner) if side == "high" else (corner, WIDE_HZ[1])
                    out.append(
                        {
                            "name": f"{bandwidth}-{side}{int(depth)}-{floor}",
                            "bandwidth": bandwidth,
                            "floor": floor_db,
                            "filter": [round(band[0], 1), round(band[1], 1)],
                            "side": side,
                            "depth_db": depth,
                        }
                    )
        # the mobile on 2026-11-08: the passband ending near 1.6 kHz (its IF shift turned fully
        # left), the shallow floor, the codec driven to about −7 dBFS
        out.append(
            {
                "name": f"{bandwidth}-saturday",
                "bandwidth": bandwidth,
                "floor": FLOORS_DB["shallow"],
                "filter": [WIDE_HZ[0], 1600.0],
                "gain": 8.0,
            }
        )
        for hot in HOT_DB:
            out.append(
                {
                    "name": f"{bandwidth}-hot{int(hot)}",
                    "bandwidth": bandwidth,
                    "floor": FLOORS_DB["shallow"],
                    "gain": hot,
                }
            )
    return out


def scenario(config: dict) -> str:
    bandwidth = config["bandwidth"]
    profile, snr = PATHS[bandwidth]
    radio = [f"rx_floor_db = {config['floor']}"]
    if "filter" in config:
        radio.append(f"rx_filter_hz = [{config['filter'][0]}, {config['filter'][1]}]")
    if "gain" in config:
        radio.append(f"rx_gain_db = {config['gain']}")
    radio_lines = "\n".join(radio)
    return f'''name = "rxcost-{config["name"]}"
description = """The receive-filter review's cost of a cut (tools/rxfilter_cost.py): {config}."""
tags = ["rxcost"]
bandwidth = {bandwidth}
seconds = 1500
seed = {SEEDS[bandwidth]}

[stations.a]
callsign = "KK4ODA"
[stations.b]
callsign = "N7XYZ"
[stations.b.radio]
{radio_lines}

[path]
profile = "{profile}"
snr_db = {snr}

[script]
steps = ["test"]

[expect]
test = "complete"
clean_end = true
idle = true
'''


def summarise(results: Path) -> None:
    rows = list(csv.DictReader(results.open(encoding="utf-8")))

    def key(row: dict) -> tuple[str, int]:
        return row["base"].removeprefix("rxcost-"), int(row["seed"])

    by = {key(r): r for r in rows}
    names = sorted({k[0] for k in by}, key=lambda n: (n.split("-")[0], n))
    print(
        f"{'configuration':24s} {'passed':>6s} {'message':>9s} {'file':>9s}"
        f"  {'message x':>9s} {'file x':>7s}  (median over draws; x against uncut, same draw)"
    )
    for name in names:
        bandwidth = name.split("-")[0]
        floor = "deep" if name.endswith("-deep") else "shallow"
        draws = sorted(seed for n, seed in by if n == name)
        passed = sum(by[(name, s)]["pass"] == "True" for s in draws)

        def rate(row: dict, what: str) -> float | None:
            value = row.get(f"{what}_bps", "")
            return float(value) if value not in ("", None) else None

        ratios: dict[str, list[float]] = {"message": [], "file": []}
        rates: dict[str, list[float]] = {"message": [], "file": []}
        for seed in draws:
            row = by[(name, seed)]
            base = by.get((f"{bandwidth}-uncut-{floor}", seed))
            for what in ("message", "file"):
                value = rate(row, what)
                if value is not None:
                    rates[what].append(value)
                    reference = rate(base, what) if base else None
                    if reference:
                        ratios[what].append(value / reference)

        def median(values: list[float], fmt: str) -> str:
            return format(statistics.median(values), fmt) if values else "—"

        print(
            f"{name:24s} {passed}/{len(draws):<4d} {median(rates['message'], '9.0f')}"
            f" {median(rates['file'], '9.0f')}  {median(ratios['message'], '9.2f')}"
            f" {median(ratios['file'], '7.2f')}"
        )


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("out", type=Path)
    ap.add_argument("--seeds", type=int, default=3)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--only", help="run only configurations whose name contains this")
    ap.add_argument("--daemon", type=Path)
    ap.add_argument("--summary-only", action="store_true")
    args = ap.parse_args()
    results = args.out / "results.csv"
    if not args.summary_only:
        scenarios = args.out / "scenarios"
        scenarios.mkdir(parents=True, exist_ok=True)
        paths = []
        for config in configurations():
            if args.only and args.only not in config["name"]:
                continue
            path = scenarios / f"rxcost-{config['name']}.toml"
            path.write_text(scenario(config), encoding="utf-8")
            paths.append(str(path))
        command = [
            sys.executable,
            str(ROOT / "tools" / "session_matrix.py"),
            *paths,
            "--seeds",
            str(args.seeds),
            "--jobs",
            str(args.jobs),
            "--out",
            str(args.out / "runs"),
            "--csv",
            str(results),
            "--drop-passing-audio",
        ]
        if args.daemon:
            command += ["--daemon", str(args.daemon)]
        subprocess.run(command, check=False)
    summarise(results)


if __name__ == "__main__":
    main()
