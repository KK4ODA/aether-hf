"""The rate controller's review, Gate 3: the prototypes beside the current controller
(``aether_model.link.rate_review``, ``docs/research/rate-control/03-candidates.md``).

Bench-only. These hold that each prototype runs a session to its end on both airs, that the
engine counts a frame its receiver never detected as air spent, and that the switch changes
nothing for the current controller — whose own behaviour the granted vectors pin."""

from __future__ import annotations

import math

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import data_capacity, seq_after
from aether_model.link.granted import GrantConfig, GrantedEngine, State, _TxRecord
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container
from aether_model.link.rate import RateController
from aether_model.link.rate_review import (
    CONTROLLERS,
    DeliveryController,
    FrameOutcome,
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
        "delivery-s": RateController,  # the receiver's advice is the current controller's
        "c1": DeliveryController,  # ADR-0094: the advice is C1's too
    }
    for name, kind in kinds.items():
        a, _ = _pair(WIDE, name)
        assert type(a.rate) is kind, name
    for name in ("delivery-s", "c1"):
        a, _ = _pair(WIDE, name)
        assert isinstance(a._sender, DeliveryController), name
        assert a._sender is not a.rate, name
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


def _strong_path() -> tuple[DeliveryController, int, int, list[FrameOutcome], FrameOutcome]:
    """A 2 300 Hz controller seeded at 10 dB on an OFDM rung, a transmission there that decodes
    whole at a reading of 15 dB, and a frame of the rung above that is lost."""
    timing = phy_timing(WIDE.params)
    a, _ = _pair(WIDE, "delivery")
    ctl = a.rate
    assert isinstance(ctl, DeliveryController)
    ctl.seed(10.0)
    here, up = ctl.modes[ctl._index], ctl.modes[ctl._index + 1]
    assert ctl.floor_modes <= here < up, "both rungs OFDM"

    def frame(mode: int, decoded: bool) -> FrameOutcome:
        cap = data_capacity(timing.capacity(mode))
        return FrameOutcome(mode, timing.data_frame_s_for(mode), cap, True, decoded, False, 15.0)

    return ctl, here, up, [frame(here, True)] * 6, frame(up, False)


def test_a_clean_transmission_tries_the_rung_above_without_its_wait() -> None:
    """Every frame at the current rung decoded and the reading rates the rung above higher: the
    try goes at once, not after six full transmissions — on a strong path the wait was most of a
    session (``04-results.md`` §3.24, K5)."""
    ctl, here, up, clean, _ = _strong_path()
    air = sum(f.air_s for f in clean)
    for _ in range(5):
        waited = ctl._since_probe.get(up, 0.0) + air >= ctl._wait_s(up)
        ctl.observe_frames(clean, 15.0)
        if ctl._probing is not None:
            break
    assert ctl._probing == up
    assert not waited, "the try came by its wait, not by the clean frames"
    assert ctl.value(up) > ctl.value(here)


def test_a_failed_try_waits_its_doubled_wait_whatever_the_frames() -> None:
    """A try that failed doubles the rung's wait, and clean frames do not cut it short."""
    ctl, here, up, clean, lost = _strong_path()
    while ctl._probing is None:
        ctl.observe_frames(clean, 15.0)
    assert ctl._probing == up
    for _ in range(3):
        ctl.observe_frames([*clean[:5], lost], 15.0)
    assert ctl._probing is None and ctl.modes[ctl._index] == here
    assert ctl._probe_wait[up] == 2 * ctl.probe_every
    air = sum(f.air_s for f in clean)
    while ctl._since_probe.get(up, 0.0) + 2 * air < ctl._wait_s(up):
        ctl.observe_frames(clean, 15.0)
        assert ctl._probing is None, "a clean transmission cut a failed try's wait short"


def test_a_rungs_account_ages_with_the_air_sent_at_other_rungs() -> None:
    """Three frames of an OFDM rung that did well, then the floor's: the rung's account ages with
    the floor's air, so it no longer outweighs the prior and the rung is chosen again only by a
    try. Aged only by its own frames, three copies of one block let an OFDM rung back in on an
    NVIS path two minutes later, and it delivered two blocks of the next seventeen frames
    (``04-results.md`` §3.24)."""
    timing = phy_timing(NARROW.params)
    a, _ = _pair(NARROW, "delivery")
    ctl = a.rate
    assert isinstance(ctl, DeliveryController)
    ctl.seed(2.0)
    ofdm, top = 5, 2

    def frames(mode: int, n: int) -> list[FrameOutcome]:
        cap = data_capacity(timing.capacity(mode))
        return [FrameOutcome(mode, timing.data_frame_s_for(mode), cap, True, True, False, 2.0)] * n

    ctl.observe_frames(frames(ofdm, 3), 2.0)
    assert ctl._accounts[ofdm].frames > ctl.prior_frames
    ctl._index = ctl.modes.index(top)
    ctl._cancel_try()
    ctl.observe_frames(frames(top, 4), 2.0)
    assert ctl._accounts[ofdm].frames <= ctl.prior_frames
    assert ctl.modes[ctl._index] == top


def test_a_slave_on_its_own_floor_rung_is_granted_floor_slots(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """C1 at a slave that chose a floor rung while the master recommends OFDM says so in its
    answer's ``data_floor``: the master sizes the slot in floor frames. Before, the slave asked
    for frames of its floor rung, was granted as many OFDM frames' air, which no floor frame
    fits, and its reply never went (``04-results.md`` §3.18)."""
    a, b = _pair(WIDE, "delivery-s")
    original = GrantedEngine._burst_mode

    def burst_mode(self: GrantedEngine) -> int:
        if self is b and self._sender is not None and self._sender.snr_db is not None:
            return 3  # tone50-75, a floor rung, whatever the master recommends
        return original(self)

    monkeypatch.setattr(GrantedEngine, "_burst_mode", burst_mode)
    reply = bytes((i * 29) % 256 for i in range(300))
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    b.send(reply)
    sim.run(until=600.0)
    assert sim.delivered(0) == reply, f"{len(sim.delivered(0))} of 300 bytes"


def test_a_try_across_the_floors_edge_with_nothing_to_cut_lets_the_floor_frames_go() -> None:
    """A try across the floor's edge takes the whole transmission for new data at the try's
    rung; with nothing left to cut, the floor frames waiting for an answer go as before. They
    had been held out of every transmission, no frame went at the try's rung to decide it, and
    the master sent empty headers until the Test ran out of time (``04-results.md`` §3.21)."""
    a, b = _pair(NARROW, "delivery-s")
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    assert a.state is State.CONNECTED
    ctl = a._sender
    assert isinstance(ctl, DeliveryController)
    ctl.observe_snr(0.0)
    top = max(m for m in ctl.modes if m < ctl.floor_modes)
    ofdm = min(m for m in ctl.modes if m >= ctl.floor_modes)
    ctl._index = ctl.modes.index(top)
    ctl._probing = ofdm
    seq = a._tx_next
    a._records[seq] = _TxRecord(seq, bytes(20), top, origin=top, tx_count=1)
    a._tx_next = seq_after(seq)
    assert not a._tx_queue
    assert [f.mode for f in a._data_frames(math.inf)] == [top]
    a._tx_queue.extend(bytes(500))
    frames = a._data_frames(math.inf)
    assert frames and {f.mode for f in frames} == {ofdm}  # with data to cut, the try's


@pytest.mark.parametrize("controller", ["delivery-s", "c1"])
def test_a_stranded_frame_comes_down_under_the_senders_own_choice(controller: str) -> None:
    """A sender whose own C1 has moved to the floor while the other station still advises OFDM
    brings a block stranded at OFDM down, as it would under the advice. Under the advice alone,
    blocks cut at OFDM on ITU NVIS at 500 Hz went fourteen copies over 200 s while every new
    block went on the floor (``04-results.md`` §3.26)."""
    a, b = _pair(NARROW, controller)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    assert a.state is State.CONNECTED
    # 500 Hz: OFDM rung 5 carries 22 bytes, rung 7 36; the floor's rung 1 (tone-36) 33
    first, seventh, own = 5, 7, 1
    assert data_capacity(a.timing.capacity(own)) == 33 and a.timing.is_floor(own)

    def stranded(mode: int, body_len: int) -> int:
        seq = a._tx_next
        a._records[seq] = _TxRecord(
            seq, bytes(body_len), mode, origin=mode, tx_count=a.cfg.max_combines + 1
        )
        a._tx_next = seq_after(seq)
        return seq

    a._recommended = a._peer_rung = seventh
    # without the sender's own choice the advice keeps both where they are
    a._own_rung = None
    kept = [stranded(first, 22), stranded(seventh, 36)]
    a._data_frames(math.inf)
    assert [a._records[s].mode for s in kept] == [first, seventh]
    # with it, the one the floor's rung 1 carries whole is re-encoded there, the other split
    # (two pieces at rung 1 or rung 0: the more robust, ADR-0086)
    a._own_rung = own
    a._data_frames(math.inf)
    assert [a._records[s].mode for s in kept] == [own, 0]
    assert a._records[kept[0]].pieces is None and a._records[kept[1]].pieces is not None
    assert a.stats.frames_reencoded >= 1 and a.stats.frames_split >= 1


# ── D3: a family refused (the second review, rate-control-2/03-candidates.md §2.3) ──────


def _d3_pair(on: bool = True) -> tuple[GrantedEngine, GrantedEngine]:
    timing = phy_timing(NARROW.params)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(NARROW.params),
            max_mode=NARROW.n_rungs - 1,
            max_burst_s=26.0,
            disc_drain_s=1e9,
            family_refusal=on,
        )

    return (
        GrantedEngine("W4ODA", timing, config(), seed=11),
        GrantedEngine("KK4XYZ", timing, config(), seed=12),
    )


def _transmission(ctl: DeliveryController, mode: int, decoded: int, frames: int = 6) -> None:
    ctl.observe_frames(
        [
            FrameOutcome(
                mode, ctl.frame_air_s[mode], ctl.capacity[mode], True, k < decoded, False, None
            )
            for k in range(frames)
        ],
        0.0,
    )


def test_d3_refuses_ofdm_when_the_choice_leaves_it_after_ofdm_delivered_nothing() -> None:
    """OFDM frames that delivered nothing refuse the family once the choice is a floor rung — the
    account has then judged every OFDM rung worse than the floor on what OFDM just did; an OFDM
    frame that delivers lifts it. While the choice stays on OFDM nothing is refused. Off,
    nothing ever is."""
    a, _ = _d3_pair()
    ctl = a._sender
    assert ctl is not None and ctl.family_refusal
    ctl.seed(6.0)
    ctl._index = ctl.modes.index(9)
    _transmission(ctl, 9, decoded=0)
    choice = ctl.modes[ctl._index]
    assert ctl.refuses(9) == (choice < 5)
    ctl._index = ctl.modes.index(2)  # the choice on the floor
    assert ctl.refuses(5) and ctl.refuses(14) and not ctl.refuses(2)
    _transmission(ctl, 6, decoded=1)
    ctl._index = ctl.modes.index(2)
    assert not ctl.refuses(6)
    off, _ = _d3_pair(on=False)
    assert off._sender is not None
    off._sender.seed(6.0)
    _transmission(off._sender, 5, decoded=0)
    off._sender._index = 0
    assert not off._sender.refuses(5)


def test_d3_refuses_ofdm_over_the_familys_echo_limit(monkeypatch: pytest.MonkeyPatch) -> None:
    """With the family's echo limit, a path that reads over it keeps every OFDM rung off — the
    start too: from a 0 dB reading the prior rates OFDM rung 6 best on a 7 ms echo, where the
    floor's tone rungs carry and OFDM's 5 ms prefix does not (``01-evidence.md`` §3)."""
    import aether_model.link.granted as g

    monkeypatch.setattr(g, "FAMILY_ECHO_LIMIT", {NARROW.n_rungs: 0.004})
    a, _ = _d3_pair()
    ctl = a._sender
    assert ctl is not None
    assert all(ctl.echo_limit[m] == pytest.approx(0.004) for m in ctl.modes if m >= 5)
    assert ctl.echo_limit[3] == pytest.approx(0.0038)  # the floor's own limits stand
    assert ctl.first_mode(0.0) >= 5
    ctl.observe_echo(0.012, 0.0)
    assert ctl.refuses(5) and ctl.first_mode(0.0) < 5
    # the advice, the current controller, is held to the same limit
    assert a.rate.echo_limit[9] == pytest.approx(0.004)
    off, _ = _d3_pair(on=False)
    assert off._sender is not None and 9 not in off._sender.echo_limit


def test_d3_brings_a_block_stuck_at_a_refused_family_down_at_once() -> None:
    """A block cut at OFDM goes on at its rung for its copies while the family is not refused
    (``max_combines``, ADR-0085); once it is, the block comes down at its next transmission —
    whole to the floor's rung that fits it, or in pieces (ADR-0086) — not after its copies:
    HARQ cannot rescue a frame whose family the path does not carry."""
    for on in (False, True):
        a, b = _d3_pair(on)
        sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
        a.connect("KK4XYZ")
        sim.run(until=60.0)
        assert a.state is State.CONNECTED and a._sender is not None
        seqs = []
        for mode, body in ((5, 22), (7, 36)):
            seq = a._tx_next
            a._records[seq] = _TxRecord(seq, bytes(body), mode, origin=mode, tx_count=1)
            a._tx_next = seq_after(seq)
            seqs.append(seq)
        a._recommended = a._peer_rung = 7
        a._own_rung = 1  # the sender's own choice has gone to the floor
        _transmission(a._sender, 5, decoded=0)
        a._sender._index = a._sender.modes.index(1)
        a._data_frames(math.inf)
        modes = [a._records[s].mode for s in seqs]
        if on:
            assert all(a.timing.is_floor(m) for m in modes), modes
            assert a._records[seqs[1]].pieces is not None  # 36 bytes fit no floor rung ≤ 1
        else:
            assert modes == [5, 7]
