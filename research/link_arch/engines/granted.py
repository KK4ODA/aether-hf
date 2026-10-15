"""The model's granted-slots engine (``aether_model.link.granted``) as a research arm.

Step 1 of ADR-0069 §5 judges the model engine where the decision was measured: the review's
adversarial simulator, its profiles and scenarios, against protocol 7 and the prototype it was
built from (``gs.py``). This module only adapts it to that simulator — the reasons it keys, the
frame names, what it believes — and gives it each profile's figures, as every arm gets them
(``setups.KNOBS``): the stations' turnaround, the slack, the decode budgets.
"""

from __future__ import annotations

import sys
from typing import Any

from aether_model.frame.modes import AirInterface
from aether_model.link.granted import GrantConfig, GrantedEngine, Role
from aether_model.link.harness import bandwidth_capabilities
from aether_model.link.phy import Container, PhyTiming, TxFrame

GRANT_KINDS = ("hdr", "ans", "disc", "disc_ack")


def config_for(air: AirInterface, knobs: tuple[float, ...], arm: str = "granted") -> GrantConfig:
    """The engine as every arm is configured: the air's ladder, the 26 s key limit, and the
    profile's figures for the protocol's constants (both stations hold the same). The bulk
    workload disconnects as it sends, so the disconnect's minute (ADR-0072) is lifted: the
    campaigns measure the transfer, as they did before it."""
    cfg = GrantConfig(
        max_mode=air.n_rungs - 1,
        capabilities=bandwidth_capabilities(air.params),
        max_burst_s=26.0,
        turnaround_s=knobs[0],
        slack_s=knobs[1],
        disc_drain_s=float("inf"),
    )
    if len(knobs) > 2:
        cfg.decode_budget_s = (knobs[2], knobs[3])
    if "_f" in arm:
        cfg.family_readings = int(arm.rsplit("_f", 1)[1])
    if "_a" in arm:
        cfg.answer_floor_margin_db = float(arm.rsplit("_a", 1)[1])
    return cfg


class ResearchGranted(GrantedEngine):
    """``GrantedEngine`` saying why it keys (``tx_reasons``) and naming its frames."""

    def __init__(self, my_call: str, timing: PhyTiming, cfg: GrantConfig, seed: int) -> None:
        self.tx_reasons: list[tuple[str, str, float | None]] = []
        super().__init__(my_call, timing, cfg, seed=seed)

    def _transmit(self, frames: list[TxFrame], **kw: Any) -> float:
        chain: list[str] = []
        f = sys._getframe(1)
        while f is not None and len(chain) < 8:
            if f.f_code.co_filename.endswith("link/granted.py"):
                chain.append(f.f_code.co_name)
            f = f.f_back
        self.tx_reasons.append((self._cause, ">".join(reversed(chain)), None))
        return super()._transmit(frames, **kw)

    @staticmethod
    def frame_kind(frame: TxFrame) -> str:
        if frame.container is Container.DATA:
            names = ["data", "connect_req", "connect_ack", "beacon", "probe", "probe_ack"]
            k = frame.payload[0] >> 5 if frame.payload else 0
            return names[k] if k < len(names) else f"data{k}"
        return GRANT_KINDS[frame.payload[0] >> 6] if frame.payload else "control"


def belief(engine: Any) -> dict[str, Any]:
    up = engine.state.value in ("connected", "disconnecting")
    return {
        "state": engine.state.value,
        "role": engine.role.value,
        "holder": up and engine.role is Role.MASTER,
    }


def build(
    timing: PhyTiming, air: AirInterface, seed: int, knobs: tuple[float, ...], arm: str = "granted"
) -> tuple[Any, Any]:
    """``arm``: ``granted``; ``granted_f<n>`` for :attr:`GrantConfig.family_readings` = n,
    ``granted_a<x>`` for :attr:`GrantConfig.answer_floor_margin_db` = x."""
    return (
        ResearchGranted("A1AAA", timing, config_for(air, knobs, arm), seed),
        ResearchGranted("B2BBB", timing, config_for(air, knobs, arm), seed + 1),
    )
