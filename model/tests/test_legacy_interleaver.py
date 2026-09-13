"""Legacy interleaver (``fec/interleaver.py``) — roadmap P1-3 replaces it."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.fec.interleaver import FrequencyInterleaver, TimeInterleaver

# ── Interleaver ───────────────────────────────────────────────────────


def test_interleavers_are_bijections(rng: np.random.Generator) -> None:
    bits = rng.integers(0, 2, 4096).astype(np.int8)
    ti = TimeInterleaver(4096, depth=8)
    assert np.array_equal(ti.deinterleave(ti.interleave(bits)), bits)
    fi = FrequencyInterleaver(52)
    b52 = bits[:52]
    assert np.array_equal(fi.deinterleave(fi.interleave(b52)), b52)


@pytest.mark.audit
@audit_xfail(
    "§2 fec/interleaver.py",
    "time 'interleaver' permutes contiguous 512-bit blocks: zero burst spreading",
)
def test_time_interleaver_spreads_a_burst() -> None:
    """A burst of 64 consecutive channel errors must not land on more than 8 adjacent
    coded bits anywhere in the codeword."""
    n = 4096
    ti = TimeInterleaver(n, depth=8)
    positions = ti.deinterleave(np.arange(n))  # channel index → codeword index
    burst = np.sort(positions[1000:1064])
    longest_run = int(
        np.max(np.diff(np.flatnonzero(np.diff(burst) != 1), prepend=-1, append=len(burst) - 1))
    )
    assert longest_run <= 8
