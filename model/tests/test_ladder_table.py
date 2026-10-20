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
    # 500 Hz rung 8 (8PSK 2/3) carries as much as rung 9 (16QAM 1/2), 53 bytes, from more SNR:
    # off the front, and the rate controller's own rule, which counts the same data — after
    # the 3-byte header (ADR-0073) — agrees
    usable = usable_modes(NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S)
    assert 8 not in usable and front == sorted(usable, key=NARROW_AWGN_THRESHOLD_DB.__getitem__)
    # tone4x100-93 (ADR-0081) is on it, between rung 3 and the first OFDM rung
    assert 8 not in front and {3, 4, 5} <= set(front)
    gaps = {(g.low, g.high): g for g in ladder_table.gaps(rungs, "awgn")}
    gap = gaps[(4, 5)]
    assert round(gap.db, 1) == round(NARROW_AWGN_THRESHOLD_DB[5] - NARROW_AWGN_THRESHOLD_DB[4], 1)
    assert gap.ratio > 1.15
    # the front is ordered by threshold and every neighbour pair is a gap
    assert (
        len(ladder_table.gaps(rungs, "moderate")) == len(ladder_table.front(rungs, "moderate")) - 1
    )
