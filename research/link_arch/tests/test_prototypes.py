"""Invariants of the prototype engines and the simulator, on seeded runs.

    uv run pytest research/link_arch/tests -q

Not part of the model's suite (``model/tests``): these hold research code to what its
reports claim. The safety invariant is checked on the air, not in an engine's beliefs: the
two transmitters never radiate at the same time.
"""

from __future__ import annotations

import pytest
from research.link_arch.advsim import AdvSim
from research.link_arch.engines.core import Ctl, Kind
from research.link_arch.engines.gs import GAP_STEPS
from research.link_arch.scenarios import SCENARIOS
from research.link_arch.scenarios import one as run_scenario
from research.link_arch.setups import builder
from research.link_arch.workload import run, script


def _run(arm: str, kind: str, trial: int, **kw: object) -> dict[str, object]:
    return run(builder(arm, **kw), kind, trial, nbytes=3000)  # type: ignore[arg-type]


def test_control_frame_round_trips_in_seven_bytes() -> None:
    ctl = Ctl(
        Kind.ANS,
        37,
        xid=2,
        count=5,
        data_floor=True,
        fam=True,
        flag=False,
        base=200,
        bitmap=0xA5C3,
        snr_db=-12.0,
        rung=19,
        n=7,
    )
    raw = ctl.encode()
    assert len(raw) == 7
    assert Ctl.decode(raw) == ctl


def test_gap_levels_fit_two_bits() -> None:
    assert len(GAP_STEPS) == 4
    assert list(GAP_STEPS) == sorted(GAP_STEPS)


@pytest.mark.parametrize("arm", ["gs", "fc"])
@pytest.mark.parametrize("kind", ["bulk", "exchange", "chat"])
@pytest.mark.parametrize(
    ("bandwidth", "channel", "snr", "profile"),
    [
        (2300, "good", 6.0, "field"),
        (500, "moderate", 0.0, "field"),
        (500, "poor", 3.0, "hostile-c"),
        (2300, "moderate", -3.0, "hostile"),
    ],
)
def test_prototypes_never_radiate_together(
    arm: str, kind: str, bandwidth: int, channel: str, snr: float, profile: str
) -> None:
    """Safety: no instant at which both transmitters radiate, whatever the channel, the
    decode latency, the keying jitter within the slack, the phantoms or the daemon's holds."""
    for trial in range(3):
        row = _run(
            arm, kind, trial, bandwidth=bandwidth, channel=channel, snr_db=snr, profile=profile
        )
        assert row["rf_overlaps"] == 0, (trial, row["_classes"])


@pytest.mark.parametrize("arm", ["gs", "fc"])
def test_delivery_is_exact_and_once(arm: str) -> None:
    """Retransmissions and late decodes never deliver a byte twice or out of order: what
    each station received is exactly what the other was given, in order."""
    for trial in range(3):
        row = _run(
            arm, "exchange", trial, bandwidth=2300, channel="moderate", snr_db=3.0, profile="field"
        )
        assert row["ok"] == 1
        assert row["clean_end"] == 1


def test_scripts_are_seeded_by_the_trial_alone() -> None:
    assert script("chat", 4) == script("chat", 4)
    assert script("exchange", 4) != script("exchange", 5)


def test_a_late_slave_transmission_is_dropped_at_the_gate() -> None:
    """The forfeit rule is enforced where the radio is keyed: a slave that decodes the
    master's header too late for its slot does not key at all (``AdvSim.dropped``)."""
    _a, b, sim = builder("gs", bandwidth=2300, channel="good", snr_db=6.0, profile="field")(0)
    assert isinstance(sim, AdvSim)
    # every frame of the master's third header-and-data transmission decoded 12 s late: past
    # the end of the transmission (about 7 s) and its gap, so past the slot it grants
    sim.scenario = lambda ctx: ("late", 12.0) if ctx.tx == 0 and ctx.tx_n == 3 else None
    from research.link_arch.workload import Driver

    drv = Driver(sim, script("bulk", 0, 2000), "B2BBB")
    drv.start()
    sim.run(until=600.0)
    assert sim.dropped[1] + b.stats.forfeits >= 1
    assert not sim.overlaps() or all(r <= 0 for *_, r in sim.overlaps())


@pytest.mark.parametrize("name", [s.name for s in SCENARIOS])
def test_granted_slots_scenarios_have_no_overlap(name: str) -> None:
    """Every deterministic adversary: granted slots keys over nothing."""
    for trial in range(2):
        row = run_scenario((name, "gs", 2300, trial, "good"))
        assert float(row.get("rf_overlaps") or 0) == 0, (name, trial)


def test_the_adr_0068_engine_overlaps_on_a_lost_handover() -> None:
    """The counter-test: the same adversary that granted slots survives makes ADR-0068's
    engine key over the other station (the TLA+ trace ImplicitTurn_forfeit_perfect)."""
    overlaps = sum(
        float(run_scenario(("handover-lost", "stated", 2300, t, "good")).get("rf_overlaps") or 0)
        for t in range(4)
    )
    assert overlaps > 0
