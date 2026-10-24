"""Cross-validation vectors for the rate controller review's prototype in the port (Gate 3).

    python tools/make_rate_review_vectors.py [--out core/aether-link/tests/data/rate_review_vectors.json]

The granted engine's own trace sessions (``tools/make_granted_vectors.py``), run with C2 — the
outer loop to a derived target, ``aether_model.link.rate_review.OuterLoopController`` — as each
station's controller: every input and every action, which ``core/aether-link/tests/
granted_vectors.rs`` replays input by input against ``aether_link::rate_review``. Bench-only: the
harness's A/B of the prototype (``docs/research/rate-control/04-results.md``) is only as good as
the port's agreement with the model.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import make_granted_vectors as granted
from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import DataKind, GrantFrame, GrantKind, data_capacity, decode_data
from aether_model.link.granted import GrantConfig, GrantedEngine
from aether_model.link.harness import phy_timing
from aether_model.link.phy import Container, TxFrame
from aether_model.link.rate_review import (
    NARROW_CLASS_PENALTY_DB,
    WIDE_CLASS_PENALTY_DB,
    DeliveryController,
    FrameOutcome,
    OuterLoopController,
)
from aether_model.link.sim import TwoStationSim

CONTROLLERS = ("olla", "delivery", "delivery-s")
"""The prototypes the port has: C2, C1, and C1 at the sender."""
TRACED = ("olla", "delivery")
"""Those traced alone (C1 at the sender is C1's controller, fed by the sender's copies)."""


def controller_trace(
    air: AirInterface, seed: int, controller: str, steps: int = 400
) -> dict[str, object]:
    """The prototype alone, input by input: a seed, then frames at the recommended rung and
    others (decoded, failed, never detected, combined), echo readings and bare SNRs drawn at
    random, and what it recommends and holds after each — every rule of the controller, where a
    session's next transmission would hide one (an echo reading's choice is taken again at the
    next transmission's)."""
    timing = phy_timing(air.params)
    ctl = GrantedEngine("TOOL", timing, GrantConfig(controller=controller)).rate
    assert isinstance(ctl, (OuterLoopController, DeliveryController))
    modes = sorted(ctl.thresholds)
    rng = random.Random(seed)
    snr = rng.uniform(-10.0, 20.0)
    ops: list[dict[str, object]] = []

    def state() -> dict[str, object]:
        common = {"recommend": ctl.recommend(), "snr": ctl.snr_db, "echo": ctl.echo}
        if isinstance(ctl, OuterLoopController):
            return {
                **common,
                "floor_offset": ctl._offset.get(True),
                "offset": ctl._offset.get(False),
            }
        return {
            **common,
            "estimates": [ctl.estimate(m) for m in ctl.modes],
            "values": [ctl.value(m) for m in ctl.modes],
        }

    ctl.seed(snr)
    ops.append({"seed": snr, **state()})
    for _ in range(steps):
        snr = max(-25.0, min(30.0, snr + rng.gauss(0.0, 2.0)))
        roll = rng.random()
        if roll < 0.6:
            frames = []
            for _ in range(rng.randint(1, 6)):
                mode = ctl.recommend() if rng.random() < 0.75 else rng.choice(modes)
                detected = rng.random() < 0.85
                decoded = detected and rng.random() < 0.6
                frames.append(
                    FrameOutcome(
                        mode,
                        timing.data_frame_s_for(mode),
                        data_capacity(timing.capacity(mode)),
                        detected,
                        decoded,
                        detected and rng.random() < 0.15,
                        snr if detected else None,
                    )
                )
            reading = snr if rng.random() < 0.9 else None
            ctl.observe_frames(frames, reading)
            ops.append(
                {
                    "frames": [
                        [f.mode, f.air_s, f.capacity, f.detected, f.decoded, f.combined]
                        for f in frames
                    ],
                    "snr_in": reading,
                    **state(),
                }
            )
        elif roll < 0.85:
            share = rng.choice([None, rng.uniform(0.0, 0.012)])
            ctl.observe_echo(share, snr)
            ops.append({"echo_in": share, "snr_in": snr, **state()})
        elif roll < 0.9 and isinstance(ctl, DeliveryController):
            # the control frames' families the engine says are in use (C1's exchange price)
            families = (rng.random() < 0.5, rng.random() < 0.5)
            ctl.families = families
            ops.append({"families": list(families), **state()})
        else:
            reading = snr if rng.random() < 0.8 else None
            ctl.observe_snr(reading)
            ops.append({"observe_snr": reading, **state()})
    return {
        "controller": controller,
        "air": air.params.bandwidth.value,
        "thresholds": [ctl.thresholds[m] for m in modes],
        "payload": [timing.capacity(m) for m in modes],
        "capacity": [data_capacity(timing.capacity(m)) for m in modes],
        "frame_s": [timing.data_frame_s_for(m) for m in modes],
        "floor_modes": timing.floor_modes,
        "penalties": [list(ctl.penalties.get(m, (0.0, 0.0, 0.0))) for m in modes]
        if isinstance(ctl, DeliveryController)
        else [],
        "echo_limit": [(timing.echo_limit or {}).get(m) for m in modes],
        # C1 ranks per second of a whole exchange (control frames [floor, ordinary], the gaps,
        # a full transmission's frames by rung); its families stay the data's in a trace
        "control_s": [ctl.control_s[True], ctl.control_s[False]]
        if isinstance(ctl, DeliveryController) and ctl.control_s
        else [],
        "gap_s": ctl.gap_s if isinstance(ctl, DeliveryController) else 0.0,
        "burst_frames": [ctl.burst_frames.get(m, 0) for m in modes]
        if isinstance(ctl, DeliveryController)
        else [],
        "targets": (
            [ctl._target[True], ctl._target[False]] if isinstance(ctl, OuterLoopController) else []
        ),
        "modes": list(ctl.modes),
        "ops": ops,
    }


def unheard(
    air: AirInterface, seed: int, snr: float
) -> granted.Callable[[], list[granted.Recorder]]:
    """Every third data frame of the transmissions to one station never detected: what the
    engine counts as air spent from the header's count (``_outcomes``), which the other sessions'
    pipes, detecting every frame, never exercise."""

    def run() -> list[granted.Recorder]:
        a, b, ra, rb = granted._pair(air)
        rng = random.Random(seed)
        count = [0]

        def lost(rx: int, container: Container, _t0: float) -> bool:
            if container is not Container.DATA or rx != 1:
                return False
            count[0] += 1
            return count[0] % 3 == 0

        sim = TwoStationSim(a, b, snr_db=snr, seed=seed, unheard=lost)
        granted._exchange(sim, a, b, granted._message(rng, 2000), granted._message(rng, 300))
        return [ra, rb]

    return run


def lopsided(
    air: AirInterface, seed: int, snr: float
) -> granted.Callable[[], list[granted.Recorder]]:
    """The slave's answers heard 10 dB under everything else: the master asks for them on the
    floor while its own headers stay ordinary, so the two families of an exchange differ — what
    C1 prices a transmission at any rung by (``granted._price``)."""

    def run() -> list[granted.Recorder]:
        a, b, ra, rb = granted._pair(air)
        rng = random.Random(seed)

        def weaker(frame: TxFrame) -> float:
            if frame.container is Container.DATA:
                return 0.0
            return -10.0 if GrantFrame.decode(frame.payload).kind is GrantKind.ANSWER else 0.0

        sim = TwoStationSim(a, b, snr_db=snr, seed=seed, frame_snr_offset=weaker)
        granted._exchange(sim, a, b, granted._message(rng, 3000), granted._message(rng, 300))
        return [ra, rb]

    return run


def floor_in_flight(seed: int) -> granted.Callable[[], list[granted.Recorder]]:
    """A 500 Hz call read 12 dB low starts C1 on the floor, and every third floor data frame is
    lost, so a floor frame is always in flight when C1 at the sender tries OFDM: the try takes
    the transmission (``granted._data_frames``' ``across``) — before, it never went."""

    def run() -> list[granted.Recorder]:
        a, b, ra, rb = granted._pair(NARROW)
        rng = random.Random(seed)
        timing = phy_timing(NARROW.params)
        count = [0]

        def offset(frame: TxFrame) -> float:
            if frame.container is not Container.DATA:
                return 0.0
            kind = decode_data(frame.payload)[0].kind
            if kind is DataKind.CONNECT_REQ:
                return -12.0
            if kind is DataKind.DATA and timing.is_floor(frame.mode):
                count[0] += 1
                return -40.0 if count[0] % 3 == 0 else 0.0
            return 0.0

        sim = TwoStationSim(a, b, snr_db=8.0, seed=seed, frame_snr_offset=offset)
        granted._exchange(sim, a, b, granted._message(rng, 4000), granted._message(rng, 200))
        return [ra, rb]

    return run


EXTRA = {
    "unheard-wide": unheard(WIDE, 41, 10.0),
    "unheard-narrow": unheard(NARROW, 42, 4.0),
    "lopsided-narrow": lopsided(NARROW, 43, 6.0),
    "lopsided-wide": lopsided(WIDE, 44, 4.0),
    "floor-in-flight": floor_in_flight(45),
}
"""Sessions of this file's own."""

SESSIONS = (
    "clean-wide",
    "weak-narrow",
    "good-wide-field",
    "moderate-narrow-field",
    "poor-wide-field",
    "outage",
    "phantom-storm",
    "ramp",
    "stranded",
    "echo-path",
    "cut-header",
    "unheard-wide",
    "unheard-narrow",
    "lopsided-narrow",
    "lopsided-wide",
    "floor-in-flight",
)
"""The sessions that build their stations with ``_pair`` and exercise the controller: a clean
path, a weak one, the three fading channels, an outage, false detections, a ramp, stranded
frames, an echo, a header cut short — and two of this file's own, with frames never detected."""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-link/tests/data/rate_review_vectors.json")
    args = ap.parse_args()
    sessions = []
    for controller in CONTROLLERS:
        granted.PROTOTYPE["controller"] = controller
        for name in SESSIONS:
            recorders = {**granted.SESSIONS, **EXTRA}[name]()
            sessions.append(
                {"name": f"{controller}:{name}", "stations": [r.doc() for r in recorders]}
            )
            print(f"{controller}:{name}: {sum(len(r.trace) for r in recorders)} steps")
    document = {
        "note": "Generated by tools/make_rate_review_vectors.py from the Python reference model: "
        "the granted engine's trace sessions with the rate controller review's prototypes C2 "
        "('olla') and C1 ('delivery') at both stations, and direct traces of each controller. "
        "Payloads are indices into blobs.",
        "blobs": granted.BLOBS.table,
        "sessions": sessions,
        "class_penalty_db": {
            str(len(table)): [list(table[m]) for m in sorted(table)]
            for table in (WIDE_CLASS_PENALTY_DB, NARROW_CLASS_PENALTY_DB)
        },
        "controller_traces": [
            controller_trace(air, seed, controller)
            for controller in TRACED
            for air, seed in ((WIDE, 1), (NARROW, 2), (WIDE, 3))
        ],
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(document, separators=(",", ":")) + "\n", encoding="utf-8", newline="\n"
    )
    print(f"wrote {out} ({out.stat().st_size // 1024} kB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
