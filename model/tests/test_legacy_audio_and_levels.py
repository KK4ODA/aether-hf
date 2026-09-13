"""Legacy audio path (``audio/soundcard.py``) and speed-level table — roadmap P1-8 / P1-3."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.audio.soundcard import AudioInterface
from aether_model.constants import BASEBAND_RATE, CP_DEFAULT_SAMPLES, DATA_CARRIERS_W, FFT_SIZE_W
from aether_model.dsp.ofdm import OFDMDemodulator, OFDMModulator
from aether_model.phy.constellation import constellation
from aether_model.speed_levels import BITS_PER_SYMBOL, WIDE_LEVELS
from aether_model.speed_levels import Modulation as LegacyModulation
from aether_model.waveform import Modulation


def test_resampler_round_trip_preserves_length() -> None:
    bb = np.exp(2j * np.pi * 500 * np.arange(1200) / BASEBAND_RATE)
    audio = AudioInterface._upsample(bb)
    assert len(audio) == 4 * len(bb)
    assert len(AudioInterface._downsample(audio)) == len(bb)


@pytest.mark.audit
@audit_xfail(
    "§2 audio/soundcard.py", "real() of a DC-centred complex baseband folds ±k subcarriers together"
)
def test_ofdm_symbol_survives_the_audio_path(rng: np.random.Generator) -> None:
    mod = OFDMModulator("wide")
    mod._window[:] = 1.0  # isolate the audio defect from the windowing defect
    bits = rng.integers(0, 2, DATA_CARRIERS_W * 2).astype(np.uint8)
    tx = mod.modulate(constellation(Modulation.QPSK).map(bits))
    audio = AudioInterface._upsample(tx)
    rx = AudioInterface._downsample(audio)[: FFT_SIZE_W + CP_DEFAULT_SAMPLES]
    rx *= np.abs(tx).max() / np.abs(rx).max()
    got = constellation(Modulation.QPSK).hard(OFDMDemodulator("wide").demodulate(rx))
    assert np.array_equal(got, bits)


@pytest.mark.audit
@audit_xfail(
    "§2 speed_levels.py", "listed net rates are not derivable from the waveform (off by 2–2.5×)"
)
def test_speed_table_rates_follow_from_the_waveform() -> None:
    symbol_rate = BASEBAND_RATE / (FFT_SIZE_W + CP_DEFAULT_SAMPLES)
    for level in WIDE_LEVELS:
        if level.modulation in (LegacyModulation.FSK2, LegacyModulation.FSK4):
            continue
        raw = level.carriers * BITS_PER_SYMBOL[level.modulation] * level.code_rate * symbol_rate
        # a net rate between 50 % and 100 % of raw is plausible overhead; anything else is not
        assert 0.5 * raw <= level.net_rate_bps <= raw, level
