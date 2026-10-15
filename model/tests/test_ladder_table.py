"""tools/ladder_table.py: each air's ladder, read from the model and the baselines, never typed."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import ladder_table  # noqa: E402
from aether_model.frame.modes import NARROW, WIDE  # noqa: E402
from aether_model.link.frames import data_capacity  # noqa: E402
from aether_model.link.rate import (  # noqa: E402
    AWGN_THRESHOLD_DB,
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_FRAME_S,
    NARROW_PAYLOAD_BYTES,
    usable_modes,
)


def test_the_table_is_the_models_ladder_and_the_rate_table() -> None:
    for air, awgn in ((NARROW, NARROW_AWGN_THRESHOLD_DB), (WIDE, AWGN_THRESHOLD_DB)):
        rungs = ladder_table.ladder(air)
        assert [r.index for r in rungs] == [r.index for r in air.ladder]
        for row, rung in zip(rungs, air.ladder, strict=True):
            assert row.payload_bytes == rung.payload_bytes
            assert row.data_bytes == data_capacity(rung.payload_bytes)
            assert row.net_bps == rung.net_bps
            assert row.threshold_db["awgn"] == awgn[rung.index]
            # every rung has its three classes in the calibrated fading pipe
            assert set(row.threshold_db) == set(ladder_table.CHANNELS)


def test_the_front_counts_the_data_a_frame_carries() -> None:
    rungs = ladder_table.ladder(NARROW)
    front = ladder_table.front(rungs, "awgn")
    # 500 Hz rung 4 (OFDM QPSK 1/3: 15 bytes in 1.054 s against tone4x100-75's 75 in 5.36 s)
    # carries 91 bit/s after the 3-byte header to rung 3's 107, from 7 dB further up: off the
    # front, and the rate controller's own rule, which counts the same data, agrees (ADR-0073)
    usable = usable_modes(NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S)
    assert 4 not in usable and front == sorted(usable, key=NARROW_AWGN_THRESHOLD_DB.__getitem__)
    assert 4 not in front and {3, 5} <= set(front)
    gaps = {(g.low, g.high): g for g in ladder_table.gaps(rungs, "awgn")}
    gap = gaps[(3, 5)]
    assert round(gap.db, 1) == round(NARROW_AWGN_THRESHOLD_DB[5] - NARROW_AWGN_THRESHOLD_DB[3], 1)
    assert gap.ratio > 1.5
    # the front is ordered by threshold and every neighbour pair is a gap
    assert (
        len(ladder_table.gaps(rungs, "moderate")) == len(ladder_table.front(rungs, "moderate")) - 1
    )
