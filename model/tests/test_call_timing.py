"""A call's own timing, and a call that lasts (ADR-0129): when each try of a call nobody answers
starts, as the engine reckons it, held to what the engine does through the simulator; the gaps a
scanning station's dwell and linger come from; and ``call_s``, the fewest tries that keep starting
for that long."""

from __future__ import annotations

import itertools

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.granted import MAX_CALL_TRIES, GrantConfig, GrantedEngine, State
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.sim import TwoStationSim, control_thresholds_for

HEARD = {"none": None, "strong": 15.0, "weak": -14.0}


def _pair(
    air: AirInterface, *, seed: int = 29, **cfg: float
) -> tuple[GrantedEngine, GrantedEngine, TwoStationSim]:
    timing = phy_timing(air.params)

    def config() -> GrantConfig:
        c = GrantConfig(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        for key, value in cfg.items():
            setattr(c, key, value)
        return c

    a = GrantedEngine("W4ODA", timing, config(), seed=seed)
    b = GrantedEngine("KK4XYZ", timing, config(), seed=seed + 1)
    sim = TwoStationSim(
        a, b, snr_db=-80.0, seed=seed, control_thresholds=control_thresholds_for(timing)
    )
    return a, b, sim


def _call_nobody_answers(
    a: GrantedEngine, sim: TwoStationSim, heard: float | None, call_s: float | None = None
) -> tuple[list[float], float]:
    """Each try's start on the air, and the pause's random part as the engine drew it."""
    first = len(sim.radiated(0))
    a.connect("KK4XYZ", heard_db=heard, call_s=call_s)
    t, jitter = sim.t, 0.0
    while a.state is not State.IDLE and t < sim.t + 400.0:
        t += 0.05
        sim.run(until=t)
        if a._connect_tries == len(a._call_plan) - 1:
            jitter = a._call_jitter
    starts = [s for s, _ in sim.radiated(0)[first:]]
    return starts, jitter


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize("heard", list(HEARD))
@pytest.mark.parametrize("tries", [2, 3, 6, 15])
def test_the_offsets_are_the_tries_the_engine_makes(
    air: AirInterface, heard: str, tries: int
) -> None:
    """Every gap before the pause to a microsecond; the pause's, which has a random part of up
    to the last try's length, at most the offset's (its longest) and at least that less the
    random part's room."""
    a, _, sim = _pair(air, call_tries=tries, turnaround_s=0.2, answer_gap_s=0.4)
    starts, jitter = _call_nobody_answers(a, sim, HEARD[heard])
    want = a.call_offsets(a._call_plan)
    assert len(starts) == len(want) == len(a._call_plan)
    got = [s - starts[0] for s in starts]
    assert got[:-1] == pytest.approx(want[:-1], abs=1e-6)
    room = a._try_s(a._call_plan[-1])
    pause = want[-1] - got[-2]
    drawn = jitter - a.cfg.call_pause_s
    assert got[-1] - got[-2] == pytest.approx(pause - room + drawn, abs=1e-6)


def test_the_gaps_are_the_longest_of_every_call() -> None:
    a, _, _ = _pair(WIDE)
    before, longest = a.call_gaps()
    seen_before = seen = 0.0
    for n in range(2, MAX_CALL_TRIES + 1):
        for heard in HEARD.values():
            starts = a.call_offsets(a._plan_for(heard, n))
            gaps = [y - x for x, y in itertools.pairwise(starts)]
            seen = max(seen, *gaps)
            if len(gaps) > 1:
                seen_before = max(seen_before, *gaps[:-1])
    assert (before, longest) == (seen_before, seen)
    # a gap before the pause is a try and its slot; the pause's holds the 15 s silence
    assert before < a.cfg.call_pause_s < longest


@pytest.mark.parametrize("call_s", [10.0, 27.0, 54.0, 100.0])
def test_a_call_asked_to_last_makes_the_fewest_tries_that_do(call_s: float) -> None:
    a, _, sim = _pair(WIDE)
    n = a.tries_lasting(call_s)
    starts = a.call_offsets(a._plan_for(None, n))
    assert starts[-2] - starts[0] >= call_s
    fewer = a.call_offsets(a._plan_for(None, n - 1))
    assert n == a.cfg.call_tries or fewer[-2] - fewer[0] < call_s
    tried, _ = _call_nobody_answers(a, sim, None, call_s=call_s)
    assert len(tried) == n
    assert tried[-2] - tried[0] >= call_s - 1e-6


def test_a_call_without_call_s_is_the_setting() -> None:
    a, _, _ = _pair(WIDE, call_tries=6)
    assert a.tries_lasting(None) == 6
    assert a.tries_lasting(0.0) == 6
    # never fewer than the setting, however short
    assert a.tries_lasting(1.0) == 6


def test_a_call_asked_to_last_too_long_makes_the_most_tries() -> None:
    a, _, _ = _pair(WIDE)
    assert a.tries_lasting(10_000.0) == MAX_CALL_TRIES
    assert a.tries_lasting(10_000.0, -14.0) == MAX_CALL_TRIES
