"""Random sessions through the scenario harness, judged only on what must never happen.

    python tools/soak.py [--count 50] [--seed 1] [--jobs 3] [--out runs/soak]
                         [--daemon core/target/release/aetherd] [--keep-passing]

The scenarios in ``bench/scenarios/`` are the situations somebody thought of. This draws
situations nobody did: a seeded mix of every fading profile the model has, SNRs from below the
floor to strong, offsets and drift up to an old rig's on 15 m, clock offsets, travel time and
long-path echoes, QSB and band closings, the other signals on the band, static, AGCs, VOX
holds, both bandwidths and a station in each — and an operator who connects, sends, aborts,
disconnects both at once, loses the path mid-transfer, or runs a Winlink program and a scanning
gateway on the host ports.

Whether a message got through is not judged: on a path below the floor it should not. What is
judged is what no path excuses (``session_matrix.judge``): a daemon that exits or panics, a
key held past the watchdog, a host program told ``CONNECTED`` while it was not listening, and
— after the script has disconnected and waited — a station not back to idle. Each failing
scenario's TOML is kept beside its run, so ``session_matrix.py <that file>`` reproduces it.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from session_matrix import DEFAULT_DAEMON, ROOT, run_one

PROFILES = [
    "awgn",
    "good",
    "moderate",
    "poor",
    "nvis",
    "flutter",
    "low-quiet",
    "low-moderate",
    "low-disturbed",
    "high-quiet",
    "high-moderate",
    "high-disturbed",
]


def toml_value(v: object) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int | float):
        return repr(v)
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(x)}" for k, x in v.items()) + " }"
    raise TypeError(v)


def to_toml(doc: dict, prefix: str = "") -> str:
    plain = {k: v for k, v in doc.items() if not isinstance(v, dict) or k == "echo"}
    plain = {
        k: v for k, v in plain.items() if not (isinstance(v, list) and v and isinstance(v[0], dict))
    }
    out = [f"{k} = {toml_value(v)}" for k, v in plain.items()]
    for k, v in doc.items():
        if isinstance(v, dict) and k != "echo":
            name = f"{prefix}{k}"
            out.append(f"\n[{name}]")
            out.append(to_toml(v, name + "."))
        elif isinstance(v, list) and v and isinstance(v[0], dict):
            for item in v:
                out.append(f"\n[[{prefix}{k}]]")
                out.append(to_toml(item, f"{prefix}{k}."))
    return "\n".join(line for line in out if line != "")


def radio(r: random.Random) -> dict:
    out: dict = {"tx_delay_ms": r.choice([0, 20, 60]), "rx_recovery_ms": r.choice([50, 150, 280])}
    if r.random() < 0.2:
        out["vox_hold_ms"] = r.choice([300, 600])
    if r.random() < 0.3:
        out["agc"] = r.choice(["fast", "auto", "slow"])
    return out


def scenario(index: int, r: random.Random) -> dict:
    bandwidth = r.choice([500, 2300])
    profile = r.choice(PROFILES)
    snr = round(r.uniform(-8.0, 20.0), 1)
    path: dict = {"profile": profile, "snr_db": snr, "cfo_hz": round(r.uniform(-80, 80), 1)}
    if r.random() < 0.3:
        path["cfo_drift_hz_per_s"] = round(r.uniform(-0.3, 0.3), 2)
    if r.random() < 0.2:
        path["sro_ppm"] = round(r.uniform(-120, 120), 0)
    if r.random() < 0.5:
        path["delay_ms"] = round(r.uniform(2, 80), 0)
    if r.random() < 0.15:
        path["echo"] = {"delay_ms": round(r.uniform(10, 120), 0), "db": round(r.uniform(-12, -3))}
    if r.random() < 0.25:
        t, points = 0.0, [[0.0, 0.0]]
        for _ in range(r.randint(2, 8)):
            t += r.uniform(10, 80)
            points.append([round(t, 1), round(r.uniform(-30, 0), 1)])
        path["level"] = points
    if r.random() < 0.3:
        path["b_to_a"] = {"snr_db": round(snr + r.uniform(-8, 8), 1)}
    stations = {
        "a": {"callsign": "KK4ODA", "radio": radio(r)},
        "b": {"callsign": "W4XYZ", "radio": radio(r)},
    }
    for who in ("a", "b"):
        st = stations[who]
        if st["radio"].get("vox_hold_ms"):
            other = stations["b" if who == "a" else "a"]
            other["answer_gap_ms"] = r.choice([0, 500, 800])
        if r.random() < 0.15:
            st["bandwidth"] = 2800 - bandwidth  # the other one
        if r.random() < 0.15:
            st["wait_for_clear"] = False
    qrm = []
    for _ in range(r.choice([0, 0, 1, 2])):
        kind = r.choice(["ofdm-arq", "pactor", "rtty", "ft8"])
        q = {
            "kind": kind,
            "at": r.choice(["a", "b", "both"]),
            "audio_hz": round(r.uniform(400, 2900)),
            "power_db": round(r.uniform(-15, 3)),
            "profile": r.choice(["good", "moderate", "poor"]),
        }
        if kind == "ft8":
            q["count"] = r.randint(3, 15)
        qrm.append(q)
    n = r.choice([200, 1500, 6000, 15000])
    host = r.random() < 0.25
    if host:
        for st in stations.values():
            st["host"] = True
        steps = [
            "host a client",
            "host b trimode",
            "host-connect",
            "host-reply 120",
            f"host-send {n}",
            "host-reply 200",
            "host-disconnect",
        ]
    else:
        steps = r.choice(
            [
                ["connect", f"message {n}", "reply 300", "disconnect"],
                [
                    "probe",
                    "connect",
                    f"send {n}",
                    f"wait {r.randint(5, 60)}",
                    "abort",
                    "connect",
                    "message 500",
                    "disconnect",
                ],
                [
                    "connect",
                    f"send {n}",
                    f"wait {r.randint(5, 60)}",
                    f"outage {r.randint(30, 300)}",
                    "wait_idle 600",
                    "wait_clear",
                ],
                ["connect", f"send {n}", f"wait {r.randint(5, 60)}", "disconnect both"],
                ["connect", f"send {n}", f"wait {r.randint(5, 40)}", "disconnect b"],
                ["test"],
            ]
        )
    # whatever happened, the operator leaves and the stations are given time to settle
    steps += ["disconnect", "wait_idle 300"]
    doc = {
        "name": f"soak-{index:04d}",
        "description": "drawn by tools/soak.py",
        "tags": ["soak"],
        "bandwidth": bandwidth,
        "seconds": 3000,
        "seed": index + 1,
        "transfer_s": 400,
        "connect_within_s": 180,
        "stations": stations,
        "path": path,
        "script": {"steps": steps, "continue_on_failure": True},
        "expect": {"idle": True},
    }
    if qrm:
        doc["qrm"] = qrm
    if r.random() < 0.2:
        doc["crashes"] = {
            "at": "both",
            "rate_per_s": round(r.uniform(0.1, 2.0), 2),
            "peak_db": round(r.uniform(10, 35)),
        }
    if steps[0] == "test":
        doc["test"] = {"budget_s": 600}
    return doc


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--count", type=int, default=50)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--out", type=Path, default=ROOT / "runs" / "soak")
    ap.add_argument("--daemon", type=Path, default=DEFAULT_DAEMON)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    r = random.Random(args.seed)
    paths = []
    for i in range(args.count):
        doc = scenario(args.seed * 10_000 + i, r)
        path = args.out / f"{doc['name']}.toml"
        path.write_text(to_toml(doc) + "\n", encoding="utf-8")
        paths.append(path)

    def one(p: Path) -> dict:
        result = run_one(p, args.out, args.daemon, args.daemon)
        print(
            f"{'pass' if result['pass'] else 'FAIL'}  {result['scenario']}  {result['why']}",
            flush=True,
        )
        return result

    from concurrent.futures import ThreadPoolExecutor

    with ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(one, paths))
    failed = [x for x in results if not x["pass"]]
    (args.out / "soak.json").write_text(json.dumps(results, indent=1), encoding="utf-8")
    print(f"\n{len(results) - len(failed)} of {len(results)} held every invariant")
    for x in failed:
        print(f"  {x['scenario']}: {x['why']}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
