"""Prototype link engines for the architecture comparison.

Each implements the model engine's driving interface (``connect``, ``send``, ``disconnect``,
``abort``, ``tick``, ``on_frame``, ``on_preamble``, ``on_tx_done``, ``on_tx_delayed``,
``drain``, ``next_deadline``) so that the same simulators run them, and says for every
transmission why it keyed (``tx_reasons``) and what it believes (``belief()``).
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from aether_model.frame.modes import AirInterface
from aether_model.link.phy import PhyTiming


def build(
    arm: str,
    timing: PhyTiming,
    air: AirInterface,
    seed: int,
    knobs: tuple[float, ...] = (0.0, 0.3),
) -> tuple[Any, Any, Callable[[Any], dict[str, Any]]]:
    """Two engines of a prototype arm and its belief reader; ``knobs`` is (extra turnaround,
    slack), as ``setups.KNOBS`` gives every arm."""
    if arm.startswith("granted"):
        from research.link_arch.engines.granted import belief as granted_belief
        from research.link_arch.engines.granted import build as build_granted

        a, b = build_granted(timing, air, seed, knobs, arm)
        return a, b, granted_belief
    if arm.startswith("gs"):
        from research.link_arch.engines.gs import GsConfig, GsEngine, belief

        cfg_a = GsConfig.for_air(air, arm, knobs)
        cfg_b = GsConfig.for_air(air, arm, knobs)
        return (
            GsEngine("A1AAA", timing, cfg_a, seed=seed),
            GsEngine("B2BBB", timing, cfg_b, seed=seed + 1),
            belief,
        )
    if arm.startswith("fc"):
        from research.link_arch.engines.fc import FcConfig, FcEngine
        from research.link_arch.engines.fc import belief as fc_belief

        cfg_a = FcConfig.for_air(air, arm, knobs)
        cfg_b = FcConfig.for_air(air, arm, knobs)
        return (
            FcEngine("A1AAA", timing, cfg_a, seed=seed),
            FcEngine("B2BBB", timing, cfg_b, seed=seed + 1),
            fc_belief,
        )
    raise ValueError(arm)
