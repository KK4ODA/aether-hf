"""The capped call (ADR-0126, accepted for building): each try today's connect body, chosen by
what was heard, at most two tries that reach a weak path with a silent pause before the last,
answered by a short control-frame acceptance. Behind ``GrantConfig.capped_call``; off, nothing
changes (the granted vectors are byte for byte today's)."""

from __future__ import annotations

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import (
    AcceptFrame,
    GrantFrame,
    GrantKind,
    caller_code,
    is_acceptance,
)
from aether_model.link.granted import GrantConfig, GrantedEngine, State
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container, TxFrame
from aether_model.link.sim import TwoStationSim, control_thresholds_for

PAUSE_S = 15.0


def _pair(
    air: AirInterface,
    snr_db: float,
    *,
    capped: tuple[bool, bool] = (True, True),
    seed: int = 17,
    pause_s: float = PAUSE_S,
) -> tuple[GrantedEngine, GrantedEngine, TwoStationSim]:
    timing = phy_timing(air.params)

    def config(on: bool) -> GrantConfig:
        cfg = GrantConfig(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        cfg.capped_call = on
        cfg.call_pause_s = pause_s
        return cfg

    a = GrantedEngine("W4ODA", timing, config(capped[0]), seed=seed)
    b = GrantedEngine("KK4XYZ", timing, config(capped[1]), seed=seed + 1)
    sim = TwoStationSim(
        a, b, snr_db=snr_db, seed=seed, control_thresholds=control_thresholds_for(timing)
    )
    return a, b, sim


def _until_connected(sim: TwoStationSim, a: GrantedEngine, b: GrantedEngine) -> float | None:
    t = 0.0
    while t < 60.0:
        t += 0.1
        sim.run(until=t)
        if a.connected and b.connected:
            return t
        if a.state is State.IDLE:
            return None
    return None


def test_an_acceptance_is_told_from_every_session_control_frame() -> None:
    accept = AcceptFrame(
        caller_code("W4ODA"),
        200,
        level=1,
        snr_db=-12.0,
        bandwidth=1,
        turnaround_s=1.5,
        identifier_s=6.5,
    )
    assert AcceptFrame.decode(accept.encode()) == accept
    assert is_acceptance(accept.encode())
    # no session's control frame has a session field of 0: every session number the engine
    # draws has its low seven bits set
    for session in range(1, 256):
        if session & 0x7F:
            for kind in GrantKind:
                assert not is_acceptance(GrantFrame(kind, session & 0x7F).encode())


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize(
    ("heard", "plan"),
    [(None, "HH"), (15.0, "CCCS"), (-14.0, "SS")],
    ids=["unheard", "strong", "weak"],
)
def test_a_capped_call_connects_and_the_called_station_knows_who_called(
    air: AirInterface, heard: float | None, plan: str
) -> None:
    a, b, sim = _pair(air, 15.0)
    a.connect("KK4XYZ", heard_db=heard)
    assert a._call_plan == plan
    assert _until_connected(sim, a, b) is not None
    assert b.remote_call == "W4ODA" and b.my_call == "KK4XYZ"
    assert a.role.name == "MASTER" and b.role.name == "SLAVE"
    # the acceptance is a control frame, on the floor for a floor try
    first = sim.radiated(1)[0]
    expected = b.timing.control_frame_s_for(plan == "SS")
    assert first[1] - first[0] == pytest.approx(expected, abs=1e-6)


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize("heard", [None, 15.0, -14.0], ids=["unheard", "strong", "weak"])
def test_a_capped_call_nobody_hears_holds_the_channel_22_5_s_at_most(
    air: AirInterface, heard: float | None
) -> None:
    """ADR-0126 §7.2, R1′: the pause before the last try is silence, not channel time; §9: it
    has a random part of up to that try's length on top of its 15 s."""
    a, b, sim = _pair(air, -60.0)
    a.connect("KK4XYZ", heard_db=heard)
    t, pause = 0.0, 0.0
    while a.state is State.CONNECTING:
        t += 0.1
        sim.run(until=t)
        if a._connect_tries == len(a._call_plan) - 1:
            pause = a._call_jitter
    last_try = a._try_s(a._call_plan[-1])
    assert PAUSE_S <= pause <= PAUSE_S + last_try
    assert t <= 40.0 + last_try
    assert t - pause <= 22.5
    assert b.state is State.IDLE
    assert sim.radiated(1) == []
    tries = sim.radiated(0)
    assert len(tries) == len(a._call_plan)
    # the last try waits the pause after the one before it has stopped listening
    assert tries[-1][0] - tries[-2][1] >= pause


@pytest.mark.parametrize(("snr_db", "floor"), [(15.0, False), (-2.0, True)])
def test_a_both_ways_try_is_accepted_in_ofdm_only_when_read_well(
    snr_db: float, *, floor: bool
) -> None:
    """ADR-0126 §7: at −2 dB the OFDM part decodes (its threshold is −5.1 dB on the AWGN
    table) but is read under 6 dB over the OFDM control threshold, so the acceptance goes on the
    floor."""
    a, b, sim = _pair(WIDE, snr_db)
    a.connect("KK4XYZ")
    assert _until_connected(sim, a, b) is not None
    first = sim.radiated(1)[0]
    assert first[1] - first[0] == pytest.approx(b.timing.control_frame_s_for(floor), abs=1e-6)


def test_a_station_without_the_switch_does_not_answer_a_capped_call() -> None:
    a, b, sim = _pair(WIDE, 15.0, capped=(True, False))
    a.connect("KK4XYZ")
    t = 0.0
    while a.state is State.CONNECTING:
        t += 0.1
        sim.run(until=t)
    assert b.state is State.IDLE
    assert sim.radiated(1) == []


def test_data_follows_a_capped_call_intact() -> None:
    a, _, sim = _pair(WIDE, 10.0)
    a.connect("KK4XYZ")
    message = bytes(range(256)) * 4
    a.send(message)
    t = 0.0
    while len(sim.st[1].delivered) < len(message) and t < 120.0:
        t += 0.1
        sim.run(until=t)
    assert bytes(sim.st[1].delivered) == message


@pytest.mark.parametrize("tries", [2, 6, 8, 10, 15])
def test_the_number_of_tries_is_the_setting(tries: int) -> None:
    """ADR-0126 §7.4: N both-ways tries, N floor tries, or three quick tries and N−1 floor
    tries; a call nobody hears goes every one, with the pause before the last."""
    a, b, sim = _pair(WIDE, -60.0)
    a.cfg.call_tries = tries
    a.connect("KK4XYZ", heard_db=-14.0)
    assert a._call_plan == "S" * tries
    a.abort()
    sim.run(until=1.0)
    t = sim.t
    a.connect("KK4XYZ", heard_db=15.0)
    assert a._call_plan == "CCC" + "S" * (tries - 1)
    a.abort()
    sim.run(until=t + 1.0)
    first = len(sim.radiated(0))
    a.connect("KK4XYZ")
    assert a._call_plan == "H" * tries
    t = sim.t
    while a.state is not State.IDLE and t < 600.0:
        t += 0.5
        sim.run(until=t)
    assert len(sim.radiated(0)) - first == tries
    assert b.state is State.IDLE


def _ofdm_control_lost(frame: TxFrame) -> float:
    """An OFDM control frame that does not get back — an echo the caller's try came through, or
    a VOX hold that swallows it whole. (The simulator loses any frame that overlaps a hold, the
    floor's too; the harness's receiver reads a tone frame whose start a hold took.)"""
    return -40.0 if frame.container is Container.CONTROL and not frame.floor else 0.0


def test_a_both_ways_try_heard_again_is_answered_on_the_floor() -> None:
    """ADR-0126 §9: a called station that hears the same both-ways try again knows its OFDM
    acceptance was lost, and answers on the floor."""
    a, b, sim = _pair(WIDE, 15.0)
    sim.frame_snr_offset = _ofdm_control_lost
    a.connect("KK4XYZ")
    assert a._call_plan == "HH"
    assert _until_connected(sim, a, b) is not None
    answers = sim.radiated(1)
    ofdm, floor = (b.timing.control_frame_s_for(f) for f in (False, True))
    assert answers[0][1] - answers[0][0] == pytest.approx(ofdm, abs=1e-6)
    assert answers[1][1] - answers[1][0] == pytest.approx(floor, abs=1e-6)
    assert a._connect_tries == 2


def test_a_quick_try_heard_again_is_still_answered_in_ofdm() -> None:
    """ADR-0126 §9: a quick try's caller listens for an OFDM acceptance only."""
    a, b, sim = _pair(WIDE, 15.0)
    sim.frame_snr_offset = _ofdm_control_lost
    a.connect("KK4XYZ", heard_db=15.0)
    assert a._call_plan == "CCCS"
    t = 0.0
    while a._connect_tries < 3 and t < 60.0:
        t += 0.1
        sim.run(until=t)
    ofdm = b.timing.control_frame_s_for(False)
    answers = sim.radiated(1)
    assert len(answers) >= 2
    assert all(end - start == pytest.approx(ofdm, abs=1e-6) for start, end in answers[:2])


def test_two_callers_draw_their_pauses_apart() -> None:
    """ADR-0126 §9: the random part comes from each engine's own draws."""
    pauses = set()
    for seed in (1, 2, 3, 4):
        a, _, sim = _pair(WIDE, -60.0, seed=seed)
        a.connect("KK4XYZ")
        t = 0.0
        while a._connect_tries < 1 or "connect" not in a._deadlines:
            t += 0.1
            sim.run(until=t)
        pauses.add(round(a._call_jitter, 3))
        assert PAUSE_S <= a._call_jitter <= PAUSE_S + a._try_s("H")
    assert len(pauses) == 4
