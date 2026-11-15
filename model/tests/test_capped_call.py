"""The capped call (ADR-0126, proposed): a call in control frames, its tries chosen by what was
heard, given up within 20 s, the caller's connect body at the head of its data. A bench switch
(``GrantConfig.capped_call``) until the author decides; off, nothing changes (the granted vectors
are byte for byte today's)."""

from __future__ import annotations

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import (
    CAPPED_VERSION,
    AcceptFrame,
    CallFrame,
    ConnectBody,
    GrantFrame,
    GrantKind,
    call_codes,
    call_family,
)
from aether_model.link.granted import GrantConfig, GrantedEngine, State
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.sim import TwoStationSim, control_thresholds_for


def _pair(
    air: AirInterface, snr_db: float, *, capped: tuple[bool, bool] = (True, True), seed: int = 17
) -> tuple[GrantedEngine, GrantedEngine, TwoStationSim]:
    timing = phy_timing(air.params)

    def config(on: bool) -> GrantConfig:
        cfg = GrantConfig(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        cfg.capped_call = on
        return cfg

    a = GrantedEngine("W4ODA", timing, config(capped[0]), seed=seed)
    b = GrantedEngine("KK4XYZ", timing, config(capped[1]), seed=seed + 1)
    sim = TwoStationSim(
        a, b, snr_db=snr_db, seed=seed, control_thresholds=control_thresholds_for(timing)
    )
    return a, b, sim


def _run(sim: TwoStationSim, a: GrantedEngine, b: GrantedEngine, limit: float) -> float | None:
    """Run until both are in the session and the called station has the caller's details, or
    the caller gives up; when, or ``None``."""
    t = 0.0
    while t < limit:
        t += 0.1
        sim.run(until=t)
        if a.connected and b.connected and b.identified:
            return t
        if a.state is State.IDLE:
            return None
    return None


def test_a_call_and_an_acceptance_are_told_from_every_session_control_frame() -> None:
    called, caller = call_codes("KK4ODA-1")
    call = CallFrame(
        called,
        caller,
        200,
        level=2,
        count=1,
        data_floor=True,
        fam=True,
        bandwidth=1,
        turnaround_s=0.3,
    )
    assert CallFrame.decode(call.encode()) == call
    assert call_family(call.encode()) == "call"
    accept = AcceptFrame(
        caller, 200, level=1, snr_db=-12.0, bandwidth=1, turnaround_s=1.5, identifier_s=6.5
    )
    assert AcceptFrame.decode(accept.encode()) == accept
    assert call_family(accept.encode()) == "accept"
    # no session's control frame has a session field of 0: every session number the engine
    # draws has its low seven bits set
    for session in range(1, 256):
        if session & 0x7F:
            for kind in GrantKind:
                assert call_family(GrantFrame(kind, session & 0x7F).encode()) is None


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize(
    ("heard", "plan"),
    [(None, "HH"), (15.0, "CCCS"), (-14.0, "SS")],
    ids=["unheard", "strong", "weak"],
)
def test_a_capped_call_connects_and_the_called_station_learns_who_called(
    air: AirInterface, heard: float | None, plan: str
) -> None:
    a, b, sim = _run_call(air, 15.0, heard)
    assert a._call_plan == plan
    took = _run(sim, a, b, 60.0)
    assert took is not None
    assert b.remote_call == "W4ODA" and b.my_call == "KK4XYZ"
    assert a.role.name == "MASTER" and b.role.name == "SLAVE"


def _run_call(
    air: AirInterface, snr_db: float, heard: float | None
) -> tuple[GrantedEngine, GrantedEngine, TwoStationSim]:
    a, b, sim = _pair(air, snr_db)
    a.connect("KK4XYZ", heard_db=heard)
    return a, b, sim


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
@pytest.mark.parametrize("heard", [None, 15.0, -14.0], ids=["unheard", "strong", "weak"])
def test_a_capped_call_nobody_hears_gives_the_channel_back_within_20_s(
    air: AirInterface, heard: float | None
) -> None:
    a, b, sim = _run_call(air, -60.0, heard)
    t = 0.0
    while a.state is State.CONNECTING:
        t += 0.1
        sim.run(until=t)
    assert t <= 20.0
    assert b.state is State.IDLE
    assert sim.radiated(1) == []


@pytest.mark.parametrize(("snr_db", "floor"), [(15.0, False), (-2.0, True)])
def test_a_both_ways_try_is_accepted_in_ofdm_only_when_read_well(
    snr_db: float, *, floor: bool
) -> None:
    """ADR-0126 §2.2: at −2 dB the OFDM part decodes (its threshold is −5.1 dB on the AWGN
    table) but is read under 6 dB over it, so the acceptance goes on the floor."""
    a, b, sim = _run_call(WIDE, snr_db, None)
    took = _run(sim, a, b, 60.0)
    assert took is not None
    first = sim.radiated(1)[0]
    length = first[1] - first[0]
    timing = b.timing
    assert length == pytest.approx(timing.control_frame_s_for(floor), abs=1e-6)


def test_details_for_another_station_end_the_session() -> None:
    _, b, sim = _run_call(WIDE, 15.0, None)
    t = 0.0
    while not (b.connected and not b.identified) and t < 10.0:
        t += 0.05
        sim.run(until=t)
    assert b.connected and not b.identified
    forged = ConnectBody(
        "W4ODA", "N0BODY", caps=b.cfg.capabilities, version=CAPPED_VERSION
    ).encode()
    assert b._take_details(forged) is None
    assert b.state is State.IDLE


def test_a_station_without_the_switch_does_not_hear_a_capped_call() -> None:
    a, b, sim = _pair(WIDE, 15.0, capped=(True, False))
    a.connect("KK4XYZ")
    t = 0.0
    while a.state is State.CONNECTING:
        t += 0.1
        sim.run(until=t)
    assert b.state is State.IDLE
    assert sim.radiated(1) == []


def test_data_follows_the_details_intact() -> None:
    a, b, sim = _run_call(WIDE, 10.0, None)
    message = bytes(range(256)) * 4
    a.send(message)
    t = 0.0
    while len(sim.st[1].delivered) < len(message) and t < 120.0:
        t += 0.1
        sim.run(until=t)
    assert bytes(sim.st[1].delivered) == message
    assert b.remote_call == "W4ODA"
