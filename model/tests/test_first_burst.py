"""ADR-0128 (proposed, off by default): the first burst after a call starts at the four channels'
mean prior's rung and tries, with one frame, the rung today's prior would have started at."""

from __future__ import annotations

from typing import Any

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import data_capacity
from aether_model.link.granted import GrantConfig, GrantedEngine, State
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container
from aether_model.link.rate import FrameOutcome
from aether_model.link.sim import TwoStationSim


def _engine(air: AirInterface, on: bool, call: str = "W4ODA", seed: int = 11) -> GrantedEngine:
    config = GrantConfig(
        capabilities=bandwidth_capabilities(air.params),
        max_mode=air.n_rungs - 1,
        max_burst_s=26.0,
        disc_drain_s=1e9,
        first_burst_try=on,
    )
    return GrantedEngine(call, phy_timing(air.params), config, seed=seed)


def _outcome(air: AirInterface, mode: int, decoded: bool) -> FrameOutcome:
    timing = phy_timing(air.params)
    cap = data_capacity(timing.capacity(mode))
    return FrameOutcome(mode, timing.data_frame_s_for(mode), cap, True, decoded, False, 20.0)


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
def test_off_the_seed_is_todays(air: AirInterface) -> None:
    ctl = _engine(air, False).rate
    ctl.seed(14.0)
    assert ctl.modes[ctl._index] == ctl.first_mode(14.0)
    assert ctl._probing is None and ctl._first_try is None


def test_on_the_first_rung_is_the_mean_priors_and_the_bold_one_is_tried() -> None:
    ctl = _engine(WIDE, True).rate
    bold = ctl.first_mode(20.0)
    ctl.seed(20.0)
    safe = ctl.modes[ctl._index]
    assert safe < bold, "at +20 dB the mean prior starts lower than the optimistic one"
    assert ctl._probing == ctl._first_try == bold
    assert ctl.recommend() == bold


def test_the_safe_start_is_never_above_todays() -> None:
    ctl = _engine(WIDE, True).rate
    assert ctl.safe_first_mode(-14.0) <= ctl.first_mode(-14.0)
    for snr in range(-20, 30):
        assert ctl.safe_first_mode(float(snr)) <= ctl.first_mode(float(snr))


def test_across_the_floors_edge_there_is_no_try_in_the_first_transmission() -> None:
    ctl = _engine(NARROW, True).rate
    bold, safe = ctl.first_mode(2.0), ctl.safe_first_mode(2.0)
    assert safe < ctl.floor_modes <= bold, "the floor's top against an OFDM rung"
    ctl.seed(2.0)
    assert ctl.modes[ctl._index] == safe
    assert ctl._probing is None and ctl._first_try is None


def test_a_decoded_try_frame_leaves_the_choice_and_the_clean_climb_goes_on() -> None:
    ctl = _engine(WIDE, True).rate
    ctl.seed(20.0)
    safe, bold = ctl.modes[ctl._index], ctl._first_try
    assert bold is not None
    ctl.observe_frames([_outcome(WIDE, bold, True), *[_outcome(WIDE, safe, True)] * 3], 20.0)
    assert ctl._first_try is None
    assert ctl._probe_wait[bold] == ctl.probe_every
    assert ctl.modes[ctl._index] == safe, "one frame does not move the choice"
    up = ctl.modes[ctl._index + 1]
    assert ctl._probing == up, "a clean first transmission tries the rung above at once"


def test_a_lost_try_frame_doubles_that_rungs_wait() -> None:
    ctl = _engine(WIDE, True).rate
    ctl.seed(20.0)
    safe, bold = ctl.modes[ctl._index], ctl._first_try
    assert bold is not None
    ctl.observe_frames([_outcome(WIDE, bold, False), *[_outcome(WIDE, safe, True)] * 3], 20.0)
    assert ctl._first_try is None
    assert ctl._probe_wait[bold] == 2 * ctl.probe_every
    assert ctl._since_probe[bold] == 0.0


def test_a_first_transmission_without_the_try_frame_gives_no_verdict() -> None:
    ctl = _engine(WIDE, True).rate
    ctl.seed(20.0)
    safe, bold = ctl.modes[ctl._index], ctl._first_try
    assert bold is not None
    ctl.observe_frames([_outcome(WIDE, safe, True)], 20.0)
    assert ctl._first_try is None
    assert bold not in ctl._probe_wait


def test_a_silent_first_slot_ends_the_try() -> None:
    ctl = _engine(WIDE, True).rate
    ctl.seed(20.0)
    ctl.silent(2)
    assert ctl._probing is None and ctl._first_try is None


def _first_data(engine: GrantedEngine) -> list[list[int]]:
    """Records the data frames' rungs of each transmission ``engine`` hands over in a session
    (a call's tries go in data frames too, ADR-0126)."""
    seen: list[list[int]] = []
    original = engine._transmit

    def transmit(frames: list[Any], **kw: Any) -> float:
        modes = [f.mode for f in frames if f.container is Container.DATA]
        if modes and engine.state is State.CONNECTED:
            seen.append(modes)
        return original(frames, **kw)

    engine._transmit = transmit  # type: ignore[method-assign]
    return seen


@pytest.mark.parametrize("on", [False, True], ids=["off", "on"])
def test_a_session_starts_safe_with_one_bold_frame(on: bool) -> None:
    a, b = _engine(WIDE, on), _engine(WIDE, on, "KK4XYZ", 12)
    seen = _first_data(a)
    payload = bytes((i * 37) % 256 for i in range(4000))
    sim = TwoStationSim(a, b, snr_db=20.0, seed=5)
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    sim.run(until=900.0)
    assert sim.delivered(1) == payload
    first = seen[0]
    if on:
        assert len(set(first)) == 2, f"one bold frame among safe ones: {first}"
        assert first[0] > max(first[1:]), "the try is the transmission's first new frame"
    else:
        assert len(set(first)) == 1, f"today's start, one rung: {first}"


def test_the_switch_reaches_the_controller() -> None:
    assert _engine(WIDE, True).rate.first_try
    assert not _engine(WIDE, False).rate.first_try
    assert not GrantConfig().first_burst_try
