"""Step 10.2b's bench tool (ADR-0115): the test sequence, and its analysis checked on the model.

The analysis is what Gate 2 reads every recorded radio with, so it is checked here end to end on
the FM channel model with both sound cards' clocks off and a recording that starts early: it must
find the clocks, align the sequence, and read back the curve, the band and the times the model
was built with.
"""

from __future__ import annotations

import sys
from dataclasses import replace
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import fm_bench  # noqa: E402
from aether_model.fm import channel as ch  # noqa: E402


def test_the_sequence_is_the_same_every_time_and_its_manifest_round_trips() -> None:
    m1, x1 = fm_bench.make("voice", 1)
    _, x2 = fm_bench.make("voice", 1)
    assert np.array_equal(x1, x2)
    assert fm_bench.Manifest.from_json(m1.to_json()) == m1
    assert len(x1) == round(m1.length_s * fm_bench.FS)
    assert [s.level_dbfs for s in m1.steps] == fm_bench.LEVELS_DBFS
    assert len(m1.transmissions) == 8
    assert len(m1.markers) == 8
    # nothing in it reaches full scale: a sound card's clipping is not what is measured
    assert float(np.max(np.abs(x1))) < 1.0
    _, other = fm_bench.make("voice", 2)
    assert not np.array_equal(x1, other)


def test_a_manifest_of_another_kind_is_refused() -> None:
    with pytest.raises(ValueError, match="manifest"):
        fm_bench.Manifest.from_json('{"format": "something-else/1"}')


def test_the_analysis_reads_back_what_the_model_was_built_with() -> None:
    m, _ = fm_bench.make("voice", 1)
    base = ch.wide_voice(cnr=18.0, seed=4)
    cfg = replace(
        base,
        sender_card=ch.SoundCardConfig(clock_ppm=60.0),
        transmitter=replace(base.transmitter, gain_db=20.0),
        receiver_card=ch.SoundCardConfig(clock_ppm=-90.0),
    )
    r = fm_bench.analyse(m, fm_bench.run_model(m, cfg, lead_s=0.37))
    # a fast DAC and a slow ADC both shorten what is recorded
    assert r.clock_ppm == pytest.approx(-150.0, abs=1.0)
    assert r.delay_s == pytest.approx(0.37, abs=0.002)
    # the curve: the knee inside the steps, about a decibel per decibel under it, falling over it
    sndr = [s["sndr_db"] for s in r.steps]
    i = int(np.argmax(sndr))
    assert 6 <= i <= len(sndr) - 4
    assert 24.0 < r.peak_sndr_db < 27.0
    assert (sndr[i] - sndr[i - 6]) / 9.0 == pytest.approx(1.0, abs=0.2)
    assert sndr[i] - sndr[i + 3] > 4.0
    assert r.notes == []
    # the band: emphasis undone, the high-passes under 300 Hz, the low-passes over 3 kHz
    resp = dict(zip(r.response_hz, r.response_db, strict=True))
    for f in (600.0, 1000.0, 1500.0, 2000.0):
        assert resp[f] == pytest.approx(0.0, abs=1.0)
    assert resp[200.0] < -10.0
    assert resp[3400.0] < -6.0
    # the squelch: closed with no carrier, open after the key, shut after the release
    t = cfg.transmitter
    sq = cfg.receiver.squelch
    assert sq is not None
    assert r.squelch_closes
    assert len(r.key_to_audio_s) == len(r.release_to_silence_s) == 8
    # the key is held a 20 ms block at a time, so a time can fall up to a block late
    opened = t.key_delay_s + t.attack_s / 2 + sq.open_s
    assert all(opened - 0.005 <= v <= opened + 0.025 for v in r.key_to_audio_s)
    assert all(0.15 <= v <= 0.21 for v in r.release_to_silence_s)


def test_the_flat_port_sequence_runs_through_the_flat_model() -> None:
    m, _ = fm_bench.make("flat", 1)
    base = ch.flat_port(cnr=24.0, seed=2)
    cfg = replace(base, transmitter=replace(base.transmitter, gain_db=12.0))
    r = fm_bench.analyse(m, fm_bench.run_model(m, cfg))
    assert r.clock_ppm == pytest.approx(0.0, abs=0.5)
    assert not r.squelch_closes  # a data port with no squelch
    assert r.peak_sndr_db > 20.0
