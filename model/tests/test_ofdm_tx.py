"""P1-4: OFDM symbol construction, preamble, passband conversion, frame transmitter."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import db, tone

from aether_model.channel import complex_normal
from aether_model.frame.codec import FrameCodec
from aether_model.frame.modes import LONG, MODES, SHORT
from aether_model.phy.constellation import constellation
from aether_model.phy.ofdm import OfdmDemodulator, OfdmModulator, carrier_map, zadoff_chu
from aether_model.phy.passband import AudioToBaseband, BasebandToAudio
from aether_model.phy.preamble import FrameHeader, FrameType, preamble
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import WIDE_2300, Modulation

P = WIDE_2300
FS = P.fs_baseband


# ── carrier map and sequences ─────────────────────────────────────────


def test_carrier_map_matches_waveform_params() -> None:
    cm = carrier_map(P)
    assert cm.n_carriers == 57
    assert cm.bins[0] == -28 and cm.bins[-1] == 28
    assert list(cm.pilot_carriers) == list(range(0, 57, 4))
    assert len(cm.pilot_carriers) == P.n_pilot_carriers == 15
    assert len(cm.data_carriers) == P.n_data_carriers == 42
    assert not set(cm.pilot_carriers) & set(cm.data_carriers)


def test_zadoff_chu_is_cazac() -> None:
    for n, u in ((57, 7), (29, 3), (12, 7), (68, 7)):
        zc = zadoff_chu(n, u)
        np.testing.assert_allclose(np.abs(zc), 1.0)
        acf = np.abs(np.fft.ifft(np.fft.fft(zc) * np.conj(np.fft.fft(zc))))
        assert acf[0] == pytest.approx(n)
        assert acf[1:].max() < 1e-9
    with pytest.raises(ValueError):
        zadoff_chu(57, 3)  # 3 divides 57


def test_uw_sequences_have_low_cross_correlation() -> None:
    pre = preamble(P)
    cands = pre.uw_candidates()
    assert len(cands) == 32
    worst = 0.0
    for a in cands:
        assert np.all(np.abs(cands[a]) == 1.0)
        for b in cands:
            if a < b:
                worst = max(worst, abs(np.vdot(cands[a], cands[b])) / 57)
    assert worst <= 0.3


def test_sc_symbol_has_two_identical_halves_and_unit_power() -> None:
    mod = OfdmModulator(P)
    pre = preamble(P)
    ext = mod.to_time(pre.sc_values)
    body = ext[P.cp_samples : P.cp_samples + P.fft_size]
    np.testing.assert_allclose(body[:100], body[100:], atol=1e-12)
    assert np.mean(np.abs(body) ** 2) == pytest.approx(1.0, rel=0.02)


def test_header_round_trip() -> None:
    for code in [*list(range(14)), 16]:
        assert FrameHeader.from_code(code).code == code
    assert FrameHeader(FrameType.CONTROL, 5).code == 16
    with pytest.raises(ValueError):
        FrameHeader.from_code(20)


# ── symbol round trip ─────────────────────────────────────────────────


def test_noiseless_symbol_round_trip_is_exact(rng: np.random.Generator) -> None:
    """The window/overlap-add construction must add no ICI (legacy code: 18.7 % EVM)."""
    mod, dem = OfdmModulator(P), OfdmDemodulator(P)
    qpsk = constellation(Modulation.QPSK)
    vals = [mod.symbol_values(qpsk.map(rng.integers(0, 2, 84).astype(np.uint8))) for _ in range(6)]
    x = mod.modulate(vals)
    for i, v in enumerate(vals):
        got = dem.carriers(x, i * P.symbol_samples)
        evm = np.sqrt(np.mean(np.abs(got - v) ** 2))
        assert evm < 1e-9


def test_symbol_round_trip_survives_timing_offset_within_cp(rng: np.random.Generator) -> None:
    """Any FFT start within the un-tapered CP region gives the values up to a linear phase
    ramp — the equaliser sees that ramp as part of the channel."""
    mod, dem = OfdmModulator(P), OfdmDemodulator(P)
    qpsk = constellation(Modulation.QPSK)
    vals = [mod.symbol_values(qpsk.map(rng.integers(0, 2, 84).astype(np.uint8))) for _ in range(3)]
    x = mod.modulate(vals)
    for early in (-15, -5, 5, 12):
        got = dem.carriers(x, P.symbol_samples + early)
        ratio = got / vals[1]
        np.testing.assert_allclose(np.abs(ratio), 1.0, atol=1e-9)
        phase = np.unwrap(np.angle(ratio))
        slope = np.polyfit(np.arange(57), phase, 1)[0]
        assert slope == pytest.approx(2 * np.pi * early / P.fft_size, abs=1e-9)


def test_full_pilot_symbol_has_low_papr() -> None:
    mod = OfdmModulator(P)
    ext = mod.to_time(mod.symbol_values(None, full_pilot=True))
    body = ext[P.cp_samples : P.cp_samples + P.fft_size]
    papr = db(np.max(np.abs(body) ** 2) / np.mean(np.abs(body) ** 2))
    assert papr < 4.5  # zero-padded IFFT of a CAZAC: ≈ 3.5 dB versus ≈ 10 dB for data symbols


# ── frame transmitter ─────────────────────────────────────────────────


def _frame(rng: np.random.Generator, mode_idx: int = 4):
    codec = FrameCodec(MODES[mode_idx], LONG)
    payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
    qam = codec.encode(payload)
    tx = FrameTransmitter(P)
    return codec, payload, tx.baseband(FrameHeader(FrameType.DATA, mode_idx), LONG, qam)


def test_frame_waveform_length_power_and_papr(rng: np.random.Generator) -> None:
    _, _, bb = _frame(rng)
    assert len(bb) == LONG.samples + P.taper_samples
    assert abs(db(float(np.mean(np.abs(bb) ** 2)))) < 0.5
    papr = db(np.max(np.abs(bb) ** 2) / np.mean(np.abs(bb) ** 2))
    assert 6.0 < papr < 12.0  # OFDM with 57 carriers; the PAPR study (P2-4) will act on this


def test_preamble_symbols_have_the_same_power_as_data_symbols(rng: np.random.Generator) -> None:
    """No level step between preamble and data (the ALC complaint about Mercury)."""
    _, _, bb = _frame(rng)
    per = P.symbol_samples
    powers = [np.mean(np.abs(bb[i * per : (i + 1) * per]) ** 2) for i in range(LONG.total_symbols)]
    assert max(powers) / min(powers) < 10 ** (1.0 / 10)  # within 1 dB


def test_occupied_bandwidth_and_out_of_band_emissions(rng: np.random.Generator) -> None:
    """99 % of the power inside ±1 180 Hz of the centre; ≥ 300 Hz beyond the edge carriers the
    emission is at least 40 dB below the in-band level (per Hz)."""
    _, _, bb = _frame(rng)
    conv = BasebandToAudio(P)
    audio = conv.process(np.concatenate((bb, np.zeros(1000)))).astype(np.float64)
    spec = np.abs(np.fft.rfft(audio * np.hanning(len(audio)))) ** 2
    f = np.fft.rfftfreq(len(audio), 1 / P.audio_rate)
    c = P.centre_hz
    inband = spec[(f >= c - 1180) & (f <= c + 1180)].sum()
    assert inband / spec.sum() > 0.99
    ref = np.mean(spec[(f >= c - 1000) & (f <= c + 1000)])
    for lo, hi in (
        (c + 1440, c + 1560),
        (c - 1560, c - 1440),
        (c + 2000, c + 3000),
        (c + 5000, c + 9000),
    ):
        assert db(np.mean(spec[(f >= lo) & (f < hi)]) / ref) < -40.0, (lo, hi)


# ── passband converters ───────────────────────────────────────────────


def test_passband_round_trip_is_transparent_for_in_band_tones() -> None:
    conv_tx, conv_rx = BasebandToAudio(P), AudioToBaseband(P)
    n = 16000
    x = 0.5 * tone(700.0, FS, n) + 0.5 * tone(-1100.0, FS, n)
    audio = conv_tx.process(x)
    assert audio.dtype == np.float32
    assert len(audio) == 6 * n
    y = conv_rx.process(audio)
    delay = (conv_tx.tx_delay_samples + conv_rx.rx_delay_samples) // P.resample_factor
    assert (conv_tx.tx_delay_samples + conv_rx.rx_delay_samples) % P.resample_factor == 0
    ref = x[2000 : n - 2000]
    got = y[2000 + delay : n - 2000 + delay]
    got = got * np.exp(-1j * np.angle(np.vdot(ref, got)))  # the carrier phase is arbitrary
    err = np.mean(np.abs(got - ref) ** 2) / np.mean(np.abs(ref) ** 2)
    assert db(err) < -50.0


def test_passband_converters_are_block_invariant(rng: np.random.Generator) -> None:
    x = complex_normal(rng, 6000)
    a, b = BasebandToAudio(P), BasebandToAudio(P)
    whole = a.process(x)
    split = np.concatenate([b.process(x[:1234]), b.process(x[1234:1235]), b.process(x[1235:])])
    np.testing.assert_allclose(split, whole, atol=1e-6)
    audio = whole.astype(np.float64)
    c, d = AudioToBaseband(P), AudioToBaseband(P)
    whole_bb = c.process(audio)
    split_bb = np.concatenate(
        [d.process(audio[:7001]), d.process(audio[7001:7004]), d.process(audio[7004:])]
    )
    np.testing.assert_allclose(split_bb, whole_bb, atol=1e-9)


def test_rx_converter_rejects_the_image_and_out_of_band_noise() -> None:
    conv_rx = AudioToBaseband(P)
    n = 48000
    t = np.arange(n) / P.audio_rate
    audio = np.cos(2 * np.pi * 4500.0 * t) + np.cos(2 * np.pi * 50.0 * t)  # both outside the band
    y = conv_rx.process(audio)[2000:]
    assert db(float(np.mean(np.abs(y) ** 2))) < -50.0


def test_frame_survives_the_audio_path_noiselessly(rng: np.random.Generator) -> None:
    """TX baseband → 48 kHz audio → RX baseband, then demodulate with the known delay:
    every data symbol's EVM stays below −40 dB (the legacy path had 30/104 bit errors)."""
    codec, payload, bb = _frame(rng)
    tx = FrameTransmitter(P)
    audio = tx.audio(np.concatenate((np.zeros(400), bb)))
    rx = AudioToBaseband(P)
    y = rx.process(audio.astype(np.float64))
    delay = (BasebandToAudio(P).tx_delay_samples + rx.rx_delay_samples) // P.resample_factor
    dem = OfdmDemodulator(P)
    hdr = FrameHeader(FrameType.DATA, 4)
    expected = tx.symbol_values(hdr, LONG, codec.encode(payload))
    start = 400 + delay
    first = dem.carriers(y, start)
    phase = np.exp(-1j * np.angle(np.vdot(expected[0], first)))  # arbitrary carrier phase
    for i, v in enumerate(expected):
        got = dem.carriers(y, start + i * P.symbol_samples) * phase
        evm = np.sqrt(np.mean(np.abs(got - v) ** 2) / np.mean(np.abs(v) ** 2))
        assert db(evm**2) < -40.0, i


def test_short_frame_layout_also_builds(rng: np.random.Generator) -> None:
    codec = FrameCodec(MODES[0], SHORT)
    qam = codec.encode(bytes(codec.payload_bytes))
    bb = FrameTransmitter(P).baseband(FrameHeader(FrameType.CONTROL), SHORT, qam)
    assert len(bb) == SHORT.samples + P.taper_samples
