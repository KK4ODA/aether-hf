"""The PHY benchmark tools still run against the model as it stands.

The release gate (``tools/bench_gate.py``) sweeps ``tools/bench_phy.py`` on every tag, and
nothing else ran it: when the ladder of ADR-0013 renumbered the modes, the bench asked for an
OFDM mode's layout by a rung number and the beta.51 release stopped at its gate. One frame a
point on each air is enough to catch that class of break.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import bench_phy  # noqa: E402
from aether_model.frame.modes import air_interface  # noqa: E402
from aether_model.phy.pipeline import Modem  # noqa: E402
from aether_model.waveform import NARROW_500, WIDE_2300  # noqa: E402


@pytest.mark.parametrize("params", [WIDE_2300, NARROW_500], ids=["2300", "500"])
def test_a_bench_point_runs_on_every_ofdm_mode_the_gate_and_the_curves_use(params: object) -> None:
    modem = Modem(params)  # type: ignore[arg-type]
    air = air_interface(params)  # type: ignore[arg-type]
    # the gate's modes on the wide air, the ladder's OFDM modes on the narrow one
    modes = (0, 4, 8, 13) if params is WIDE_2300 else air.ofdm_ladder[:2]
    for mode in modes:
        row = bench_phy.run_point(modem, "awgn", mode, 30.0, 1, 11)
        assert row["mode"] == mode
        assert row["decoded"] == 1, row
        assert row["throughput_bps"] == round(
            8 * modem.payload_bytes(air.modes[mode]) / air.long.duration_s
        )


@pytest.mark.parametrize("bandwidth", [2300, 500])
def test_the_chat_bench_runs_a_conversation_on_each_air(bandwidth: int) -> None:
    """``tools/bench_chat.py`` (ADR-0027) against the model as it stands: one conversation on a
    clean path under today's turn-taking, in a chat, and with the handover it measured and did
    not adopt — every line arrives, and a reply arrives sooner in a chat than when it has to
    wait to be polled."""
    import statistics

    import bench_chat

    rows = {}
    for policy in ("base", "request", "handover+request"):
        point = bench_chat.Point(
            policy,
            tuple(sorted(bench_chat.POLICIES[policy].items())),
            bandwidth,
            "awgn",
            12.0,
            1,
            fading=True,
            floor_cap=True,
            max_burst_s=bench_chat.MAX_BURST_S,
        )
        rows[policy] = bench_chat.run_session(point, 0)
        assert rows[policy]["connected"] == 1 and rows[policy]["lost"] == 0, rows[policy]
    base, chat = (rows[p]["replies"] for p in ("base", "request"))
    assert statistics.median(chat) < statistics.median(base)  # type: ignore[arg-type]
    assert rows["request"]["requests"], "the chat asked for the turn"
