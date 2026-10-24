"""The rate controller's review, Gate 3: the prototypes beside the current controller
(``aether_model.link.rate_review``, ``docs/research/rate-control/03-candidates.md``).

Bench-only. These hold that each prototype runs a session to its end on both airs, that the
engine counts a frame its receiver never detected as air spent, and that the switch changes
nothing for the current controller — whose own behaviour the granted vectors pin."""

from __future__ import annotations

from itertools import pairwise

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import data_capacity
from aether_model.link.granted import GrantConfig, GrantedEngine
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container
from aether_model.link.rate import RateController
from aether_model.link.rate_review import (
    CONTROLLERS,
    DeliveryController,
    FrameOutcome,
    OuterLoopController,
    decode_probability,
)
from aether_model.link.sim import TwoStationSim


def _pair(air: AirInterface, controller: str) -> tuple[GrantedEngine, GrantedEngine]:
    timing = phy_timing(air.params)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(air.params),
            max_mode=air.n_rungs - 1,
            max_burst_s=26.0,
            disc_drain_s=1e9,
            controller=controller,
        )

    return (
        GrantedEngine("W4ODA", timing, config(), seed=11),
        GrantedEngine("KK4XYZ", timing, config(), seed=12),
    )


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize("controller", CONTROLLERS)
def test_every_controller_carries_a_session_to_its_end(air: AirInterface, controller: str) -> None:
    a, b = _pair(air, controller)
    payload = bytes((i * 37) % 256 for i in range(3000))
    sim = TwoStationSim(a, b, snr_db=6.0, seed=5)
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    sim.run(until=1800.0)
    assert sim.delivered(1) == payload, f"{controller} delivered {len(sim.delivered(1))} bytes"


def test_the_switch_builds_the_controller_it_names() -> None:
    kinds = {
        "current": RateController,
        "delivery": DeliveryController,
        "olla": OuterLoopController,
        "delivery-s": RateController,  # the receiver's advice is the current controller's
    }
    for name, kind in kinds.items():
        a, _ = _pair(WIDE, name)
        assert type(a.rate) is kind, name
    a, _ = _pair(WIDE, "delivery-s")
    assert isinstance(a._sender, DeliveryController)
    with pytest.raises(ValueError, match="no rate controller"):
        _pair(WIDE, "nonesuch")


def test_a_frame_never_detected_is_air_spent() -> None:
    """The receiver's outcomes carry every frame the header stated, heard or not: on a path
    that loses one data frame in three before detection, the delivery account sees them."""
    a, b = _pair(WIDE, "delivery")
    seen: list[FrameOutcome] = []
    original = DeliveryController.observe_frames

    def spy(self: DeliveryController, frames: list[FrameOutcome], snr: float | None) -> None:
        seen.extend(frames)
        original(self, frames, snr)

    DeliveryController.observe_frames = spy  # type: ignore[method-assign]
    try:
        count = [0]

        def unheard(rx: int, container: Container, t0: float) -> bool:
            if container is not Container.DATA or rx != 1:
                return False
            count[0] += 1
            return count[0] % 3 == 0

        sim = TwoStationSim(a, b, snr_db=12.0, seed=2, unheard=unheard)
        payload = bytes(range(256)) * 8
        a.connect("KK4XYZ")
        a.send(payload)
        a.disconnect()
        sim.run(until=900.0)
    finally:
        DeliveryController.observe_frames = original  # type: ignore[method-assign]
    assert sim.delivered(1) == payload
    missed = [f for f in seen if not f.detected]
    assert missed, "no frame the header stated was counted unheard"
    assert all(not f.decoded and f.air_s > 0 for f in missed)


def test_the_prior_passes_ninety_per_cent_at_the_threshold() -> None:
    assert decode_probability(-3.2, -3.2, 0.5) == pytest.approx(0.9)
    assert decode_probability(-6.2, -3.2, 0.5) < 0.05
    assert decode_probability(0.0, -3.2, 0.5) > 0.99


def test_the_outer_loop_targets_derive_from_the_ladder() -> None:
    """C2's target error rate is the rate ratio of each family's neighbours, never tuned."""
    a, _ = _pair(WIDE, "olla")
    ctl = a.rate
    assert isinstance(ctl, OuterLoopController)
    for floor in (True, False):
        rungs = [m for m in ctl.modes if (m < ctl.floor_modes) == floor]
        rates = [ctl.lossless(m) for m in rungs]
        expected = sum(1 - lo / hi for lo, hi in pairwise(rates)) / (len(rates) - 1)
        assert ctl._target[floor] == pytest.approx(min(0.5, max(0.05, expected)))


def test_a_silent_step_lands_on_a_rung_the_echo_allows() -> None:
    """C1 at the sender steps down after silence as R9 does, but never onto a rung the echo
    rules out (ADR-0087): from the first OFDM rung of the 500 Hz air on a 7 ms echo, one step
    goes past rungs 3 and 4 to the floor's rung 2."""
    a, _ = _pair(NARROW, "delivery")
    ctl = a.rate
    assert isinstance(ctl, DeliveryController)
    ctl.echo = 0.012
    assert ctl._echo_out(ctl.modes.index(3)) and ctl._echo_out(ctl.modes.index(4))
    ctl._index = ctl.modes.index(5)
    ctl.silent(1)
    assert ctl.recommend() == 2
    ctl.echo = None
    ctl._index = ctl.modes.index(5)
    ctl.silent(1)
    assert ctl.recommend() == 4


def test_a_try_is_decided_on_more_frames_than_the_prior_weighs() -> None:
    """A try of the rung above runs until its own frames outweigh the prior (three, against
    ``prior_frames`` 2): one frame a transmission at the sender is a coin toss either way
    (``04-results.md`` §3.14). Nor does one lucky frame make a rung above choosable."""
    timing = phy_timing(NARROW.params)

    def fresh() -> tuple[DeliveryController, int, int]:
        a, _ = _pair(NARROW, "delivery")
        ctl = a.rate
        assert isinstance(ctl, DeliveryController)
        ctl.seed(8.0)
        return ctl, ctl.modes[ctl._index], ctl.modes[ctl._index + 1]

    def frame(mode: int) -> FrameOutcome:
        cap = data_capacity(timing.capacity(mode))
        return FrameOutcome(mode, timing.data_frame_s_for(mode), cap, True, True, False, 8.0)

    # one lucky frame above, outside a try: news of the rung, not enough to move to it
    ctl, here, up = fresh()
    ctl.observe_frames([frame(here)] * 5 + [frame(up)], 8.0)
    assert ctl.recommend() == here
    # a try of the rung above: two clean frames are not yet a verdict, the third is
    ctl, here, up = fresh()
    ctl._probing = up
    for _ in range(2):
        ctl.observe_frames([frame(here)] * 5 + [frame(up)], 8.0)
        assert ctl._probing == up, "a try of two frames is still going"
        assert ctl.modes[ctl._index] == here
    ctl.observe_frames([frame(here)] * 5 + [frame(up)], 8.0)
    assert ctl._probing != up and ctl.modes[ctl._index] == up, "three clean frames climb"


def test_the_wait_before_a_try_is_air_at_the_rung_tried() -> None:
    """The wait before a try of the rung above is :attr:`probe_every` full transmissions *at
    that rung*, in seconds of air: from the 500 Hz floor's top (four 5.36 s frames a
    transmission here) a try of OFDM (six 1.05 s frames) comes after two floor transmissions,
    not six — six floor transmissions were 160 s on the harness (``04-results.md`` §3.16)."""
    timing = phy_timing(NARROW.params)
    a, _ = _pair(NARROW, "delivery")
    ctl = a.rate
    assert isinstance(ctl, DeliveryController)
    ctl.seed(0.0)
    ctl._index = ctl.modes.index(4)
    ctl._cancel_try()
    cap = data_capacity(timing.capacity(4))
    floor = [FrameOutcome(4, timing.data_frame_s_for(4), cap, True, True, False, 0.0)] * 4
    transmissions = 0
    while ctl._probing is None and transmissions < 10:
        ctl.observe_frames(floor, 0.0)
        transmissions += 1
    assert ctl._probing == 5
    assert transmissions == 2
