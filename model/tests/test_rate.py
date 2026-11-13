"""The rate controller (ADR-0094, ADR-0099): C1 at the sender, the one controller.

These hold that a session runs to its end on both airs, that the receiver says how many frames
of a transmission it never detected (protocol 12), the controller's account, prior, tries and
silence step, the family refused (D3), and that the stuck frames' targets and the slave's slot
go by the station's own choice (D1)."""

from __future__ import annotations

import math
from dataclasses import replace

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import GrantFrame, data_capacity, seq_after
from aether_model.link.granted import GrantConfig, GrantedEngine, State, _TxRecord
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container
from aether_model.link.rate import FrameOutcome, RateController, _Account, decode_probability
from aether_model.link.sim import TwoStationSim


def _pair(air: AirInterface) -> tuple[GrantedEngine, GrantedEngine]:
    timing = phy_timing(air.params)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(air.params),
            max_mode=air.n_rungs - 1,
            max_burst_s=26.0,
            disc_drain_s=1e9,
        )

    return (
        GrantedEngine("W4ODA", timing, config(), seed=11),
        GrantedEngine("KK4XYZ", timing, config(), seed=12),
    )


@pytest.mark.parametrize("air", [WIDE, NARROW], ids=["2300", "500"])
def test_a_session_carries_to_its_end(air: AirInterface) -> None:
    a, b = _pair(air)
    payload = bytes((i * 37) % 256 for i in range(3000))
    sim = TwoStationSim(a, b, snr_db=6.0, seed=5)
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    sim.run(until=1800.0)
    assert sim.delivered(1) == payload, f"delivered {len(sim.delivered(1))} bytes"


def test_the_receiver_says_how_many_frames_it_never_detected() -> None:
    """Protocol 12 (ADR-0099): every control frame says how many data frames of the other
    station's last transmission, by a count stated in it, its sender never detected. On a path
    that loses one data frame in three before detection the sender learns of every one."""
    a, b = _pair(WIDE)
    decided: dict[float, bool] = {}  # asked at the preamble and again at the frame

    def unheard(rx: int, container: Container, t0: float) -> bool:
        if container is not Container.DATA or rx != 1:
            return False
        if t0 not in decided:
            decided[t0] = len(decided) % 3 == 2
        return decided[t0]

    sim = TwoStationSim(a, b, snr_db=12.0, seed=2, unheard=unheard)
    payload = bytes(range(256)) * 8
    a.connect("KK4XYZ")
    a.send(payload)
    a.disconnect()
    sim.run(until=900.0)
    assert sim.delivered(1) == payload
    lost = sum(decided.values())
    assert lost > 0
    assert a.stats.frames_missed == lost, (a.stats.frames_missed, lost)
    assert b.stats.frames_missed == 0  # the slave sent no data


def test_the_prior_passes_ninety_per_cent_at_the_threshold() -> None:
    assert decode_probability(-3.2, -3.2, 0.5) == pytest.approx(0.9)
    assert decode_probability(-6.2, -3.2, 0.5) < 0.05
    assert decode_probability(0.0, -3.2, 0.5) > 0.99


def test_a_silent_step_lands_on_a_rung_the_echo_allows() -> None:
    """C1 at the sender steps down after silence as R9 does, but never onto a rung the echo
    rules out (ADR-0087): from the first OFDM rung of the 500 Hz air on a 7 ms echo, one step
    goes past rungs 3 and 4 to the floor's rung 2."""
    a, _ = _pair(NARROW)
    ctl = a.rate
    assert isinstance(ctl, RateController)
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

    def fresh() -> tuple[RateController, int, int]:
        a, _ = _pair(NARROW)
        ctl = a.rate
        assert isinstance(ctl, RateController)
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
    a, _ = _pair(NARROW)
    ctl = a.rate
    assert isinstance(ctl, RateController)
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


def _strong_path() -> tuple[RateController, int, int, list[FrameOutcome], FrameOutcome]:
    """A 2 300 Hz controller seeded at 10 dB on an OFDM rung, a transmission there that decodes
    whole at a reading of 15 dB, and a frame of the rung above that is lost."""
    timing = phy_timing(WIDE.params)
    a, _ = _pair(WIDE)
    ctl = a.rate
    assert isinstance(ctl, RateController)
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


def _near_miss(**switches: bool) -> tuple[RateController, int, int]:
    """A 2 300 Hz controller at 14 dB on rung 16, three clean transmissions there, then a try of
    rung 17 whose three frames all decode: the near miss of the 2026-11-12 sessions
    (``rate-control-3/01-evidence.md`` §1), where the prior rates rung 17 at 0.03."""
    timing = phy_timing(WIDE.params)
    a, _ = _pair(WIDE)
    ctl = a.rate
    assert isinstance(ctl, RateController)
    for name, on in switches.items():
        setattr(ctl, name, on)
    ctl.seed(14.0)
    here, up = ctl.modes[ctl._index], ctl.modes[ctl._index + 1]

    def frame(mode: int) -> FrameOutcome:
        cap = data_capacity(timing.capacity(mode))
        return FrameOutcome(mode, timing.data_frame_s_for(mode), cap, True, True, False, 14.0)

    for _ in range(3):
        ctl.observe_frames([frame(here)] * 5, 14.0)
    ctl._index = ctl.modes.index(here)
    ctl._cancel_try()
    ctl._probing = up
    for _ in range(3):
        ctl.observe_frames([frame(here)] * 5 + [frame(up)], 14.0)
    return ctl, here, up


def test_the_review_switches_are_off_by_default() -> None:
    ctl = RateController()
    assert not (ctl.prior_steps_aside or ctl.clean_try_keeps_wait or ctl.try_unaged)


def test_a_clean_try_loses_to_the_prior_today() -> None:
    """The near miss as C1 has it: every frame of the try decoded, the verdict went to the rung
    below, and the wait before the next try doubled."""
    ctl, here, up = _near_miss()
    assert (here, up) == (16, 17)
    assert ctl.modes[ctl._index] == here
    assert ctl._probe_wait[up] == 2 * ctl.probe_every


def test_g3_the_prior_steps_aside_once_a_rung_has_a_whole_try() -> None:
    """G3 (``rate-control-3/03-candidates.md`` §2): the prior counts in full with no frames of
    the rung's own, half at half a try, and nothing once the rung has a whole try's worth, so
    the clean try climbs and the climb holds."""
    ctl = RateController(prior_steps_aside=True)
    assert ctl._prior_weight(_Account()) == ctl.prior_frames
    assert math.isclose(ctl._prior_weight(_Account(frames=1.5)), ctl.prior_frames / 2)
    assert ctl._prior_weight(_Account(frames=3.0)) == 0.0
    assert ctl._prior_weight(_Account(frames=9.0)) == 0.0
    assert RateController()._prior_weight(_Account(frames=9.0)) == RateController().prior_frames
    ctl, _, up = _near_miss(prior_steps_aside=True)
    assert ctl.modes[ctl._index] == up
    assert ctl._probe_wait[up] == ctl.probe_every


def test_g4_a_lost_try_whose_frames_all_decoded_keeps_its_wait() -> None:
    """G4: the clean try still loses to the prior, but its rung's wait is not doubled; a try
    that lost a frame doubles it as before."""
    ctl, here, up = _near_miss(clean_try_keeps_wait=True)
    assert ctl.modes[ctl._index] == here
    assert ctl._probe_wait[up] == ctl.probe_every
    ctl, here, up, clean, lost = _strong_path()
    ctl.clean_try_keeps_wait = True
    while ctl._probing is None:
        ctl.observe_frames(clean, 15.0)
    for _ in range(3):
        ctl.observe_frames([*clean[:5], lost], 15.0)
    assert ctl._probing is None and ctl._probe_wait[up] == 2 * ctl.probe_every


def test_g5_a_tried_rung_is_not_aged_by_the_air_sent_beside_it() -> None:
    """G5: while rung 17 is tried, rung 16's frames sent with it do not age rung 17's account;
    the try's three frames are all there at the verdict."""
    on, _, up = _near_miss(try_unaged=True)
    off, _, _ = _near_miss()
    assert on._accounts[up].frames > off._accounts[up].frames
    decay = 0.5 ** (1.0 / on.half_life)
    assert math.isclose(on._accounts[up].frames, 1.0 + decay + decay**2)


def test_g1_a_smaller_margin_starts_higher() -> None:
    """G1 (``rate-control-3/03-candidates.md`` §2), set through ``GrantConfig.rate``: at 1 dB
    under the reading the prior rates a rung higher at the same reading, so the start is higher
    on a strong path (40 m Good: today rung 16, the best single rung 17)."""
    timing = phy_timing(WIDE.params)
    config = GrantConfig(
        capabilities=bandwidth_capabilities(WIDE.params), max_mode=WIDE.n_rungs - 1
    )
    today = GrantedEngine("W4ODA", timing, config, seed=1).rate
    smaller = GrantedEngine(
        "W4ODA", timing, replace(config, rate={"prior_margin_db": 1.0}), seed=1
    ).rate
    assert smaller.prior_margin_db == 1.0 and today.prior_margin_db == 3.0
    assert smaller.first_mode(19.0) > today.first_mode(19.0)


def test_wide_only_keeps_the_review_values_off_the_500_hz_air() -> None:
    """With ``wide_only`` the review's values apply on the 2 300 Hz air alone: a station on the
    500 Hz air runs today's controller (``rate-control-3/04-results.md`` §5.3), and moving
    between airs (ADR-0026) takes the values on and off with it."""
    rate = {"prior_margin_db": 1.0, "family_gain_db": 1.0, "wide_only": True}

    def engine(air: AirInterface) -> GrantedEngine:
        config = GrantConfig(
            capabilities=bandwidth_capabilities(air.params), max_mode=air.n_rungs - 1, rate=rate
        )
        return GrantedEngine("W4ODA", phy_timing(air.params), config, seed=1)

    wide, narrow = engine(WIDE).rate, engine(NARROW).rate
    assert (wide.prior_margin_db, wide.family_gain_db) == (1.0, 1.0)
    assert (narrow.prior_margin_db, narrow.family_gain_db) == (3.0, 0.5)
    moved = engine(WIDE)
    moved.set_air(phy_timing(NARROW.params), bandwidth_capabilities(NARROW.params))
    assert moved.rate.prior_margin_db == 3.0


def test_a_rungs_account_ages_with_the_air_sent_at_other_rungs() -> None:
    """Three frames of an OFDM rung that did well, then the floor's: the rung's account ages with
    the floor's air, so it no longer outweighs the prior and the rung is chosen again only by a
    try. Aged only by its own frames, three copies of one block let an OFDM rung back in on an
    NVIS path two minutes later, and it delivered two blocks of the next seventeen frames
    (``04-results.md`` §3.24)."""
    timing = phy_timing(NARROW.params)
    a, _ = _pair(NARROW)
    ctl = a.rate
    assert isinstance(ctl, RateController)
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
    a, b = _pair(WIDE)
    original = GrantedEngine._burst_mode

    def burst_mode(self: GrantedEngine) -> int:
        if self is b and self.rate is not None and self.rate.snr_db is not None:
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
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    assert a.state is State.CONNECTED
    ctl = a.rate
    assert isinstance(ctl, RateController)
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


def test_a_stranded_frame_comes_down_under_the_senders_own_choice() -> None:
    """A block stranded at OFDM comes down once the station's own choice has gone to the floor:
    whole to the floor's rung that fits it, or in pieces. Under the old advice alone, blocks cut
    at OFDM on ITU NVIS at 500 Hz went fourteen copies over 200 s while every new block went on
    the floor (``rate-control/04-results.md`` §3.26); nothing but the station's own choice bounds
    them now (ADR-0099 §2.2)."""
    a, b = _pair(NARROW)
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

    kept = [stranded(first, 22), stranded(seventh, 36)]
    # with the choice above them both, both stay where they are
    a.rate._index = a.rate.modes.index(9)
    a._answered_rung = 9  # the choice when the last answer was read
    a._data_frames(math.inf)
    assert [a._records[s].mode for s in kept] == [first, seventh]
    # with it on the floor, the one rung 1 carries whole is re-encoded there, the other split
    # (two pieces at rung 1 or rung 0: the more robust, ADR-0086)
    a.rate._index = a.rate.modes.index(own)
    a._answered_rung = own  # an answer read since: a split goes by it, never by silence alone
    a._data_frames(math.inf)
    assert [a._records[s].mode for s in kept] == [own, 0]
    assert a._records[kept[0]].pieces is None and a._records[kept[1]].pieces is not None
    assert a.stats.frames_reencoded >= 1 and a.stats.frames_split >= 1


# ── D3: a family refused (ADR-0099 §2.3) ───────────────────────────────────────────────


def _d3_pair() -> tuple[GrantedEngine, GrantedEngine]:
    timing = phy_timing(NARROW.params)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(NARROW.params),
            max_mode=NARROW.n_rungs - 1,
            max_burst_s=26.0,
            disc_drain_s=1e9,
        )

    return (
        GrantedEngine("W4ODA", timing, config(), seed=11),
        GrantedEngine("KK4XYZ", timing, config(), seed=12),
    )


def _transmission(ctl: RateController, mode: int, decoded: int, frames: int = 6) -> None:
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
    frame that delivers lifts it. While the choice stays on OFDM nothing is refused."""
    a, _ = _d3_pair()
    ctl = a.rate
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


def test_d3_refuses_ofdm_when_the_echo_rules_out_every_ofdm_rung() -> None:
    """A path whose echo is over every OFDM rung's own limit refuses the family as a whole — no
    family-wide limit is set (ADR-0099 §2.3), but rungs' limits that cover the family do it."""
    a, _ = _d3_pair()
    ctl = a.rate
    ctl.echo_limit = {**ctl.echo_limit, **{m: 0.004 for m in ctl.modes if m >= 5}}
    assert not ctl.refuses(5)
    ctl.observe_echo(0.012, 0.0)
    assert ctl.refuses(5) and ctl.refuses(14) and not ctl.refuses(2)
    assert ctl.first_mode(0.0) < 5


def test_d3_brings_a_block_stuck_at_a_refused_family_down_at_once() -> None:
    """A block cut at OFDM goes on at its rung for its copies while the family is not refused
    (``max_combines``, ADR-0085); once it is, the block comes down at its next transmission —
    whole to the floor's rung that fits it, or in pieces (ADR-0086) — not after its copies:
    HARQ cannot rescue a frame whose family the path does not carry."""
    a, b = _d3_pair()
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    assert a.state is State.CONNECTED
    seqs = []
    for mode, body in ((5, 22), (7, 36)):
        seq = a._tx_next
        a._records[seq] = _TxRecord(seq, bytes(body), mode, origin=mode, tx_count=1)
        a._tx_next = seq_after(seq)
        seqs.append(seq)
    a.rate._index = a.rate.modes.index(9)  # not refused yet: they stay for their copies
    a._data_frames(math.inf)
    assert [a._records[s].mode for s in seqs] == [5, 7]
    _transmission(a.rate, 5, decoded=0)
    a.rate._index = a.rate.modes.index(1)  # the choice gone to the floor
    a._answered_rung = 1  # as the answer that told it said
    a._data_frames(math.inf)
    modes = [a._records[s].mode for s in seqs]
    assert all(a.timing.is_floor(m) for m in modes), modes
    assert a._records[seqs[1]].pieces is not None  # 36 bytes fit no floor rung ≤ 1


# ── D1: one controller, at the sender (ADR-0099 §2.2) ──────────────────────────────────


def test_the_slaves_slot_is_sized_and_stated_in_the_family_the_slave_states() -> None:
    """The master grants the slave's slot in frames of the family the slave says its data goes in
    (``data_floor``), and the header states that family, which the slave reckons its room in: the
    slave's room is the master's reckoning of the slot. With the old advice's rung in the header,
    a floor rung over a slot sized in OFDM frames, the slave of the final checks' Trimode run
    sent 27 s of tone frames into a slot the master reckoned at 6 s
    (``rate-control-2/04-results.md`` §4.1, ADR-0099 §2.2)."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    assert a.state is State.CONNECTED
    sized: list[int] = []
    original = a._grant_size

    def spy(rung: int) -> tuple[int, bool, int]:
        sized.append(rung)
        return original(rung)

    sent: list[tuple[float, GrantFrame]] = []
    keyed = a._key_granting

    def key(slot, frames):  # type: ignore[no-untyped-def]
        control = [f for f in frames if f.container is Container.CONTROL]
        if control:
            sent.append((slot.grant_s, GrantFrame.decode(control[0].payload)))
        keyed(slot, frames)

    a._grant_size = spy  # type: ignore[method-assign]
    a._key_granting = key  # type: ignore[method-assign]
    for floor_only in (False, True):
        a._slave_floor_only = floor_only
        a._send_header()
        assert sized and a.timing.is_floor(sized[-1]) == floor_only
        grant_s, header = sent[-1]
        assert header.slot_floor == floor_only
        # what the slave reckons its data's room in (``_schedule_slave``) fits the master's slot
        room = header.n * a._frame_s(header.slot_floor)
        assert header.n > 0 and room <= grant_s + 1e-9


# ── the echo (ADR-0087) ────────────────────────────────────────────────────────────────


def _narrow_echo() -> RateController:
    a, _ = _pair(NARROW)
    return a.rate


def test_an_echo_keeps_the_rungs_it_breaks_off_the_ladder() -> None:
    """ADR-0087: a floor reading of -3 dB is a quiet path at -3 dB or ITU's disturbed NVIS at
    any SNR, and on the second the 500 Hz air's rungs 3 and 4 do not carry. The echo the
    floor's frames read tells the two apart: read, the start and every choice keep off the
    rungs it breaks; gone, they are back on offer. (The OFDM rungs have no limit of their own:
    on such a path it is D3 that keeps the family off, by what its frames deliver.)"""
    from aether_model.link.rate import NARROW_ECHO_LIMIT

    quiet = _narrow_echo()
    assert quiet.first_mode(-3.0) >= min(NARROW_ECHO_LIMIT)
    rc = _narrow_echo()
    rc.observe_echo(0.01, -3.0)
    assert rc.first_mode(-3.0) not in NARROW_ECHO_LIMIT
    rc.seed(-3.0)
    assert rc.recommend() not in NARROW_ECHO_LIMIT
    rc.observe_echo(0.0, 5.0)
    rc.observe_echo(0.0, 5.0)
    rc.observe_echo(0.0, 5.0)
    rc.observe_echo(0.0, 5.0)
    assert not any(rc._echo_out(rc.modes.index(m)) for m in NARROW_ECHO_LIMIT)


def test_an_echo_read_at_once_takes_the_choice_down() -> None:
    """A choice the echo rules out does not wait for its frames to fail: the next transmission
    goes at the best rung the echo allows. A weak frame's reading counts for nothing
    (``echo_min_snr_db``): its noise is as large as a 4 ms echo's spill."""
    rc = _narrow_echo()
    rc.seed(-3.0)
    rc._index = rc.modes.index(4)
    rc.observe_echo(0.05, rc.echo_min_snr_db - 1.0)
    assert rc.echo is None and rc.recommend() == 4
    rc.observe_echo(0.01, -3.0)
    assert rc.recommend() not in (3, 4)
