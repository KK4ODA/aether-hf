"""P9-6: the fading lossy pipe — the channel a frame sees, and the SNR it is judged at."""

from __future__ import annotations

import math

import numpy as np
import pytest

from aether_model.frame.modes import NARROW, WIDE
from aether_model.link.fading import (
    FadingPipe,
    SharedFading,
    effective_snr_db,
    shapes_for,
    success_probability,
)
from aether_model.link.phy import Container, TxFrame


def test_the_effective_snr_lies_between_the_worst_element_and_the_mean() -> None:
    rng = np.random.default_rng(3)
    snr = rng.exponential(10.0, size=(20, 12))
    worst, mean = 10 * math.log10(snr.min()), 10 * math.log10(snr.mean())
    for beta in (0.05, 1.0, 20.0, 1e4):
        eff = effective_snr_db(snr, beta)
        assert worst - 1e-6 <= eff <= mean + 1e-6, (beta, worst, eff, mean)
    # a large β reads the mean, a small one the worst element (to within β·ln N)
    assert abs(effective_snr_db(snr, 1e6) - mean) < 0.01
    assert abs(effective_snr_db(snr, 1e-6) - worst) < 0.05


def test_the_waterfall_passes_its_ten_percent_point_at_the_threshold() -> None:
    assert success_probability(-3.0, -3.0) == pytest.approx(0.9)
    assert success_probability(-1.0, -3.0) > 0.99
    assert success_probability(-5.0, -3.0) < 0.1


@pytest.mark.parametrize("profile", ["good", "moderate", "poor"])
def test_the_shared_fade_has_unit_mean_power(profile: str) -> None:
    fading = SharedFading(profile, seed=5)
    gains = np.concatenate(
        [fading.power(t, t + 1.0, (-900.0, 0.0, 900.0)) for t in range(0, 3000, 2)]
    )
    assert gains.mean() == pytest.approx(1.0, abs=0.08)


def test_a_frame_and_the_one_after_it_share_a_slow_fade() -> None:
    """ITU Good fades over seconds: two frames a second apart see nearly the same channel,
    two a minute apart are unrelated — what the per-frame draws of the old pipe could not
    give, and what a burst followed by its acknowledgement lives with on the air."""
    fading = SharedFading("good", seed=9)
    near, far = [], []
    for t in np.arange(0.0, 4000.0, 80.0):
        a = fading.power(t, t + 1.0, (0.0,)).mean()
        near.append((a, fading.power(t + 1.0, t + 2.0, (0.0,)).mean()))
        far.append((a, fading.power(t + 60.0, t + 61.0, (0.0,)).mean()))
    r_near = np.corrcoef(np.array(near).T)[0, 1]
    r_far = np.corrcoef(np.array(far).T)[0, 1]
    assert r_near > 0.9 and abs(r_far) < 0.3, (r_near, r_far)


def test_on_awgn_a_frame_decodes_at_the_channel_snr() -> None:
    pipe = FadingPipe(
        SharedFading("awgn"), shapes_for(WIDE), lambda _: 1.0, np.random.default_rng(1)
    )
    reported, decode = pipe.judge(TxFrame(Container.DATA, b"", mode=4), 7.5, 0.0, 1.0)
    assert decode == 7.5 and abs(reported - 7.5) < 3.0


def test_frames_are_shaped_by_the_layout_they_go_out_on() -> None:
    narrow = shapes_for(NARROW)
    floor_data = narrow(TxFrame(Container.DATA, b"", mode=0))
    ordinary = narrow(TxFrame(Container.DATA, b"", mode=4))
    floor_control = narrow(TxFrame(Container.CONTROL, b"", floor=True))
    assert floor_data.duration_s > 3 * ordinary.duration_s
    assert floor_control.duration_s > narrow(TxFrame(Container.CONTROL, b"")).duration_s
    wide = shapes_for(WIDE)
    ofdm = wide(TxFrame(Container.DATA, b"", mode=WIDE.floor_modes))
    assert max(ofdm.carriers_hz) - min(ofdm.carriers_hz) > 1500 > max(ordinary.carriers_hz)
    # the tone floor is the same 400 Hz on either air
    tone = wide(TxFrame(Container.DATA, b"", mode=0))
    assert tone == floor_data
    assert len(tone.carriers_hz) == 16 and max(tone.carriers_hz) - min(tone.carriers_hz) == 375.0
    # a fast kind (ADR-0014) is sampled across its data's tones: 800 or 1 600 Hz, the frame
    # as long as the floor's
    for rung, span in ((2, 750.0), (5, 1500.0)):
        fast = wide(TxFrame(Container.DATA, b"", mode=rung))
        assert fast.duration_s == tone.duration_s
        assert len(fast.carriers_hz) == 16
        assert max(fast.carriers_hz) - min(fast.carriers_hz) == span
