"""A call nobody answers, from the model's own engine: when each try starts and what it holds.

Two link engines (`GrantedEngine`) through the model's `TwoStationSim` with the called station
unable to hear anything (-80 dB), so every call runs to "no answer". For each air, each plan
(`H`: not heard lately, both-ways tries; `S`: heard weak, floor tries) and each number of tries
2-15 (ADR-0126 §7.4), twenty seeds (the pause before the last try has a random part). Writes the
transmissions the caller keyed, the plan, the end of the call and the two parts' lengths to
`calls.json`, and says whether the two airs' schedules are the same.

    python -m uv run python call_schedules.py      # about three minutes
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "model"))
sys.path.insert(0, str(ROOT / "tools"))

import bench_link as bench  # noqa: E402
from aether_model.frame.modes import NARROW, WIDE  # noqa: E402
from aether_model.link.granted import GrantConfig, GrantedEngine, State  # noqa: E402
from aether_model.link.harness import bandwidth_capabilities, phy_timing  # noqa: E402
from aether_model.link.sim import TwoStationSim, control_thresholds_for  # noqa: E402

SEEDS = range(1, 21)
PLANS = {"H": None, "S": -15.0}  # heard_db: none, or weak


def schedule(air, tries: int, heard: float | None, seed: int) -> dict:
    timing = phy_timing(air.params)

    def config() -> GrantConfig:
        c = GrantConfig(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        c.call_tries = tries
        return c

    a = GrantedEngine("W4ODA", timing, config(), seed=seed)
    b = GrantedEngine("KK4XYZ", timing, config(), seed=seed + 1)
    sim = TwoStationSim(
        a,
        b,
        snr_db=-80.0,
        seed=seed,
        thresholds=bench.table_for(air)[0],
        control_thresholds=control_thresholds_for(timing),
    )
    a.connect("KK4XYZ", heard_db=heard)
    t, end = 0.0, None
    while t < 400:
        t = round(t + 0.05, 3)
        sim.run(until=t)
        if a.state == State.IDLE:
            end = t
            break
    return {
        "plan": "".join(a._call_plan),
        "tx": [(round(s, 3), round(e, 3)) for s, e in sim.radiated(0)],
        "end": end,
        "ofdm": a._try_s("C"),
        "floor": a._try_s("S"),
    }


def main() -> None:
    out: dict[str, list[dict]] = {}
    for air, hz in ((WIDE, 2300), (NARROW, 500)):
        for tries in range(2, 16):
            for plan, heard in PLANS.items():
                out[f"{hz}/{tries}/{plan}"] = [schedule(air, tries, heard, s) for s in SEEDS]
    Path(__file__).with_name("calls.json").write_text(json.dumps(out))
    same = all(out[k] == out[k.replace("2300/", "500/", 1)] for k in out if k.startswith("2300/"))
    print("the two airs' calls are the same:", same)
    for key in ("2300/2/H", "2300/6/H", "2300/6/S"):
        s = out[key][0]
        ends = [x["end"] for x in out[key]]
        print(key, s["plan"], [a for a, _ in s["tx"]], f"ends {min(ends)}-{max(ends)} s")


if __name__ == "__main__":
    main()
