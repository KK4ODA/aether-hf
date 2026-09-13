"""Legacy OFDM modulator/demodulator (``dsp/ofdm.py``) — roadmap P1-4/P1-6 rewrite it."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.dsp.ofdm import OFDMDemodulator, OFDMModulator, SubcarrierMap
from aether_model.phy.constellation import constellation
from aether_model.waveform import Modulation


def _qpsk_symbol(rng: np.random.Generator) -> tuple[np.ndarray, np.ndarray]:
    smap = SubcarrierMap("wide")
    bits = rng.integers(0, 2, smap.n_data * 2).astype(np.uint8)
    return bits, constellation(Modulation.QPSK).map(bits)


def test_qpsk_hard_decisions_survive_noiseless_loopback(rng: np.random.Generator) -> None:
    bits, syms = _qpsk_symbol(rng)
    rx = OFDMDemodulator("wide").demodulate(OFDMModulator("wide").modulate(syms))
    assert np.array_equal(constellation(Modulation.QPSK).hard(rx), bits)


def test_symbol_length_is_fft_plus_cp() -> None:
    mod = OFDMModulator("wide")
    assert len(mod.modulate(np.zeros(mod.smap.n_data, dtype=complex))) == 256 + 38


@pytest.mark.audit
@audit_xfail(
    "§2 dsp/ofdm.py",
    "raised-cosine window without overlap-add tapers the useful symbol: 18.7 % EVM",
)
def test_noiseless_loopback_evm_is_negligible(rng: np.random.Generator) -> None:
    _, syms = _qpsk_symbol(rng)
    rx = OFDMDemodulator("wide").demodulate(OFDMModulator("wide").modulate(syms))
    evm = np.sqrt(np.mean(np.abs(rx - syms) ** 2) / np.mean(np.abs(syms) ** 2))
    assert evm < 1e-3


@pytest.mark.audit
@audit_xfail("§2 dsp/ofdm.py", "pilot grid leaves 8 edge data carriers extrapolated")
def test_every_data_carrier_is_bracketed_by_pilots() -> None:
    smap = SubcarrierMap("wide")
    assert min(smap.data_indices) > min(smap.pilot_indices)
    assert max(smap.data_indices) < max(smap.pilot_indices)


@pytest.mark.audit
@audit_xfail("§2 constants.py", "64 carriers × 46.875 Hz = 3 000 Hz, not the 2 300 Hz wide mode")
def test_wide_mode_fits_in_2300_hz() -> None:
    smap = SubcarrierMap("wide")
    spacing = 12000 / smap.fft_size
    occupied = (
        max(smap.pilot_indices + smap.data_indices)
        - min(smap.pilot_indices + smap.data_indices)
        + 1
    ) * spacing
    assert occupied <= 2300.0
