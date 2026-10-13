"""Engines, channels and station profiles for the comparison, by name.

Every arm of a comparison is built here from the same pieces: the air's real timing
(``aether_model.link.harness.phy_timing``) with a keying latency the engine is told about,
the fading pipe of P9-6 for the channel class, and a station profile — ``ideal`` (the model
pipe's world: instant decoding, nothing between the engine and the antenna), ``field`` (what
the scenario harness and the logs say real stations do) or ``hostile`` (the same, worse).
"""

from __future__ import annotations

import csv
from collections.abc import Callable
from dataclasses import replace
from pathlib import Path
from typing import Any

import numpy as np

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.engine import LinkConfig, LinkEngine
from aether_model.link.fading import FadingPipe, SharedFading, frame_key, shapes_for
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import PhyTiming
from research.link_arch.advsim import AdvSim, ChannelCfg, StationCfg, instrument

ROOT = Path(__file__).resolve().parents[2]
FADING_CSV = ROOT / "bench" / "baselines" / "fading_pipe.csv"
FLOOR_READING_CAP_DB = {"awgn": 17.5, "good": 15.5, "moderate": 12.0, "poor": 5.0}

TX_LATENCY_S = 0.35
"""What every engine is told its keying takes (lead and the audio queued ahead of the card):
the daemon's own figure is about this (`station.rs`, 0.1 s lead and 0.25 s card)."""


def air_of(bandwidth: int) -> AirInterface:
    return WIDE if bandwidth == 2300 else NARROW


def timing_for(air: AirInterface) -> PhyTiming:
    return replace(phy_timing(air.params), tx_latency_s=TX_LATENCY_S)


def fading_pipe(air: AirInterface, channel: str, seed: int) -> FadingPipe | None:
    if channel == "flat":
        return None
    betas: dict[str, float] = {}
    with FADING_CSV.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            if int(r["bandwidth_hz"]) == air.params.bandwidth.value and r["channel"] == channel:
                betas[r["frame"]] = float(r["beta"])
    return FadingPipe(
        SharedFading(channel, seed),
        shapes_for(air),
        lambda frame: betas.get(frame_key(frame, air), 1.0),
        np.random.default_rng(seed + 99),
    )


PROFILES: dict[str, StationCfg] = {
    "ideal": StationCfg(decode_latency_s=0.0, key_lead_s=0.0),
    "field": StationCfg(
        lead_jitter_s=0.15,
        gate_prob=0.05,
        gate_s=(0.2, 1.5),
        vox_hold_s=0.0,
        rx_recovery_s=0.05,
        decode_latency_s=0.15,
        latency_tail_s=0.08,
        floor_latency_s=0.3,
        stall_prob=0.005,
        stall_s=(0.5, 3.0),
        phantom_per_min=1.0,
        phantom_trusted=0.05,
    ),
    "hostile-c": StationCfg(
        lead_jitter_s=0.6,
        vox_hold_s=0.15,
        rx_recovery_s=0.15,
        decode_latency_s=0.15,
        latency_tail_s=0.2,
        floor_latency_s=0.5,
        stall_prob=0.02,
        stall_s=(0.5, 4.0),
        phantom_per_min=4.0,
        phantom_trusted=0.1,
    ),
    "hostile": StationCfg(
        lead_jitter_s=0.6,
        gate_prob=0.15,
        gate_s=(0.3, 2.0),
        answer_gap_s=0.5,
        vox_hold_s=0.15,
        rx_recovery_s=0.15,
        decode_latency_s=0.15,
        latency_tail_s=0.2,
        floor_latency_s=0.5,
        stall_prob=0.02,
        stall_s=(0.5, 4.0),
        phantom_per_min=4.0,
        phantom_trusted=0.1,
    ),
}


KNOBS: dict[str, tuple[float, float, float, float]] = {
    "ideal": (0.0, 0.3, 0.15, 0.3),
    "field": (0.05, 0.35, 0.3, 0.65),
    "hostile": (0.3, 0.65, 0.6, 1.1),
    "hostile-c": (0.3, 0.65, 0.6, 1.1),
}
"""What every engine is told about the stations, per profile: (extra turnaround — the peer's
VOX hold and receiver recovery, which a station must wait out before keying —; slack — the
keying-time jitter its waits must absorb —; decode budget for an ordinary and a floor frame —
how long after a frame ends its decode is surely in). The same figures go to every arm: the
model's engine gets them as ``PhyTiming.turnaround_s``, ``LinkConfig.ack_margin_s`` and
``PhyTiming.detect_latency_s`` (it has one latency for both families: the ordinary one), the
prototypes as their gap, slack and decode budgets. The hostile daemon's holds and answer gap
are *not* in them: they are what that profile breaks."""


def model_engine(
    name: str,
    call: str,
    timing: PhyTiming,
    air: AirInterface,
    seed: int,
    knobs: tuple[float, ...] = (0.0, 0.3, 0.15, 0.3),
) -> Any:
    """The model's engine as each arm configures it."""
    caps = bandwidth_capabilities(air.params)
    timing = replace(timing, turnaround_s=timing.turnaround_s + knobs[0], detect_latency_s=knobs[2])
    cfg = LinkConfig(
        max_mode=air.n_rungs - 1,
        capabilities=caps,
        max_burst_s=26.0,
        ack_margin_s=max(0.4, knobs[1] + 0.1),
    )
    if name == "legacy":
        engine: Any = LinkEngine(call, timing, cfg, seed=seed)
    elif name == "stated":
        cfg.stated_answers = True
        engine = LinkEngine(call, timing, cfg, seed=seed)
    elif name == "stated6":
        from research.link_arch.engines.stated6 import Stated6Engine

        cfg.stated_answers = True
        engine = Stated6Engine(call, timing, cfg, seed=seed)
    else:
        raise ValueError(name)
    instrument(engine)
    return engine


def builder(
    arm: str,
    *,
    bandwidth: int = 500,
    channel: str = "moderate",
    snr_db: float = 0.0,
    asym_db: float = 0.0,
    profile: str = "field",
    profile_b: str | None = None,
    carrier_sense: bool = False,
) -> Callable[[int], tuple[Any, Any, AdvSim]]:
    """A function of the trial number giving two engines and their simulator."""
    air = air_of(bandwidth)
    timing = timing_for(air)

    def make(trial: int) -> tuple[Any, Any, AdvSim]:
        seed = 1000 * trial + 17
        knobs = KNOBS[profile]
        if arm in ("legacy", "stated", "stated6"):
            a = model_engine(arm, "A1AAA", timing, air, seed, knobs)
            b = model_engine(arm, "B2BBB", timing, air, seed + 1, knobs)
            belief = None
        else:
            from research.link_arch.engines import build

            a, b, belief = build(arm, timing, air, seed, knobs)
        ca = replace(PROFILES[profile], carrier_sense=carrier_sense)
        cb = replace(PROFILES[profile_b or profile], carrier_sense=carrier_sense)
        ch = ChannelCfg(
            snr_db=snr_db,
            asym_db=asym_db,
            fading=fading_pipe(air, channel, seed),
            floor_cap_db=FLOOR_READING_CAP_DB.get(channel),
        )
        sim = AdvSim(
            a, b, channel=ch, cfg_a=ca, cfg_b=cb, seed=seed, belief=belief, key_limit_s=28.0
        )
        return a, b, sim

    return make
