"""The other signals on the band and the static crashes (ADR-0042)."""

from __future__ import annotations

import numpy as np
import pytest

from aether_model.channel import noise_power_for_snr
from aether_model.qrm import (
    AtmosphericCrashes,
    Ft8Station,
    OfdmArqStation,
    PactorStation,
    RttyStation,
)

FS = 8000.0


def sources() -> list[object]:
    return [
        OfdmArqStation(FS, 200.0, -3.0, seed=1),
        PactorStation(FS, -600.0, 0.0, seed=2, profile="moderate"),
        RttyStation(FS, 900.0, 3.0, seed=3),
        AtmosphericCrashes(FS, noise_power_for_snr(0.0, 1.0, FS), rate_per_s=2.0, seed=4),
        Ft8Station(FS, -1100.0, 0.0, seed=9),
    ]


@pytest.mark.parametrize("index", range(5))
def test_a_source_is_the_same_however_the_stream_is_cut(index: int) -> None:
    whole = sources()[index].next(40_000)  # type: ignore[attr-defined]
    cut = sources()[index]
    rng = np.random.default_rng(index)
    parts = []
    left = 40_000
    while left:
        n = int(min(left, rng.integers(1, 1700)))
        parts.append(cut.next(n))  # type: ignore[attr-defined]
        left -= n
    np.testing.assert_allclose(np.concatenate(parts), whole, rtol=0, atol=1e-12)


def _occupied(x: np.ndarray, fraction: float = 0.99) -> tuple[float, float]:
    """The band holding ``fraction`` of the power, in hertz."""
    spectrum = np.abs(np.fft.fftshift(np.fft.fft(x))) ** 2
    freqs = np.fft.fftshift(np.fft.fftfreq(len(x), 1 / FS))
    cum = np.cumsum(spectrum) / spectrum.sum()
    lo = freqs[np.searchsorted(cum, (1 - fraction) / 2)]
    hi = freqs[np.searchsorted(cum, 1 - (1 - fraction) / 2)]
    return float(lo), float(hi)


@pytest.mark.parametrize(
    ("make", "offset", "half_width"),
    [
        (lambda: OfdmArqStation(FS, 200.0, 0.0, seed=5), 200.0, 1300.0),
        (lambda: PactorStation(FS, -600.0, 0.0, seed=6), -600.0, 330.0),
        (lambda: RttyStation(FS, 900.0, 0.0, seed=7), 900.0, 160.0),
        (lambda: Ft8Station(FS, -1100.0, 0.0, duty=0.6, seed=10), -1100.0, 40.0),
    ],
)
def test_a_station_sits_where_it_is_put_with_the_power_it_is_given(
    make: object, offset: float, half_width: float
) -> None:
    x = make().next(int(120 * FS))  # type: ignore[operator]
    on = np.abs(x) > 1e-6
    assert 0.05 < on.mean() < 0.95, f"keyed {on.mean():.0%} of the time"
    power = float(np.mean(np.abs(x[on]) ** 2))
    assert 0.6 < power < 1.3, f"{power:.2f} while keyed, against 1.0 at 0 dB"
    lo, hi = _occupied(x[on])
    assert offset - half_width < lo < offset < hi < offset + half_width, (lo, hi)


def test_an_arq_station_works_in_bursts_with_its_partners_answers_between() -> None:
    x = OfdmArqStation(FS, 0.0, 0.0, partner_db=-10.0, seed=8).next(int(300 * FS))
    level = np.abs(x)
    loud = (level > 0.5).mean()
    quiet = ((level > 0.03) & (level < 0.5)).mean()
    assert 0.4 < loud < 0.85, f"data bursts {loud:.0%} of the time"
    assert 0.03 < quiet < 0.3, f"acknowledgements {quiet:.0%} of the time"


def test_crashes_arrive_at_their_rate_well_above_the_floor() -> None:
    floor = noise_power_for_snr(0.0, 1.0, FS)
    crashes = AtmosphericCrashes(FS, floor, rate_per_s=1.0, peak_db=25.0, seed=9)
    x = crashes.next(int(600 * FS))
    loud = np.abs(x) ** 2 > floor * 10 ** (10.0 / 10.0)
    # count separate crashes: runs of loud samples more than 100 ms apart
    idx = np.flatnonzero(loud)
    starts = 1 + int(np.sum(np.diff(idx) > 0.1 * FS)) if len(idx) else 0
    assert 450 < starts < 750, f"{starts} crashes in 600 s at 1 a second"
    assert np.mean(np.abs(x) ** 2) < floor * 10, "between crashes there is nothing"


def test_an_ft8_station_keeps_to_its_slots() -> None:
    # every transmission starts half a second into a 15 s slot and lasts 12.64 s
    x = Ft8Station(FS, 0.0, 0.0, duty=1.0, seed=11).next(int(61 * FS))
    on = np.abs(x) > 0.5
    starts = np.flatnonzero(on[1:] & ~on[:-1]) + 1
    assert [round(s / FS, 2) for s in starts] == [0.5, 15.5, 30.5, 45.5, 60.5][: len(starts)]
    assert abs(on[: int(15 * FS)].sum() / FS - 12.64) < 0.05
