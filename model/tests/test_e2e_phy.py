"""P1-5 … P1-7: acquisition accuracy and end-to-end frame decoding through the simulator."""

from __future__ import annotations

import numpy as np
import pytest

from aether_model.channel import ChannelConfig, HfChannel, InterfererConfig, make_channel
from aether_model.frame.modes import MODES
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType
from aether_model.waveform import WIDE_2300

P = WIDE_2300
FS = P.fs_baseband
LEAD = 1200  # samples of silence before the frame in every test buffer


@pytest.fixture(scope="module")
def modem() -> Modem:
    return Modem(P)


def _payload(rng: np.random.Generator, modem: Modem, mode_idx: int) -> bytes:
    n = modem.payload_bytes(MODES[mode_idx])
    return rng.integers(0, 256, n, dtype=np.uint8).tobytes()


def _buffer(burst: np.ndarray, lead: int = LEAD, tail: int = 1200) -> np.ndarray:
    return np.concatenate((np.zeros(lead), burst, np.zeros(tail)))


# ── acquisition ───────────────────────────────────────────────────────


def test_noiseless_acquisition_is_exact(modem: Modem, rng: np.random.Generator) -> None:
    burst = modem.data_burst(_payload(rng, modem, 4), MODES[4])
    syncs = modem.detector.detect(_buffer(burst))
    assert len(syncs) == 1
    s = syncs[0]
    assert s.start == LEAD
    assert abs(s.cfo_hz) < 0.05
    assert s.header.frame_type is FrameType.DATA
    assert s.header_confidence > 2.0  # DATA vs CONTROL preamble sequences
    assert s.timing_peak > 0.95
    frame = modem.rx.receive(_buffer(burst), s)
    assert frame.mode == 4 and frame.mode_confidence > 3.0


@pytest.mark.parametrize("cfo", [-250.0, -133.3, -41.0, 0.0, 39.0, 77.7, 250.0])
def test_acquisition_at_0_db_across_cfo(modem: Modem, cfo: float, rng: np.random.Generator) -> None:
    """ADR-0002 target: timing within ±1 sample at 0 dB (3 kHz); the preamble CFO estimate
    is within ±3 Hz and the receiver's data-aided refinement brings it within ±0.5 Hz."""
    burst = modem.data_burst(_payload(rng, modem, 0), MODES[0])
    ch = make_channel(
        "awgn", snr_db=0.0, fs=FS, seed=int(abs(cfo) * 10) + 3, signal_power=1.0, cfo_hz=cfo
    )
    y = modem.detector.condition(ch.process(_buffer(burst)))
    syncs = modem.detector.detect(y)
    assert len(syncs) == 1, cfo
    s = syncs[0]
    assert abs(s.start - LEAD) <= 1, (s.start, cfo)
    assert abs(s.cfo_hz - cfo) < 3.0, (s.cfo_hz, cfo)
    assert s.header.frame_type is FrameType.DATA
    frame = modem.rx.receive(y, s)
    assert abs(frame.cfo_hz - cfo) < 0.5, (frame.cfo_hz, cfo)
    assert frame.mode == 0


@pytest.mark.parametrize("snr_3k_db", [-5.0, -3.0])
def test_low_snr_acquisition_and_decode(
    modem: Modem, snr_3k_db: float, rng: np.random.Generator
) -> None:
    """P2-3: the most robust mode must acquire and decode at −5 dB (3 kHz) with random
    offsets; the BPSK-1/5 code itself gives up around −6 dB."""
    ok = 0
    for trial in range(6):
        payload = _payload(rng, modem, 0)
        cfo = float(rng.uniform(-250, 250))
        ch = make_channel(
            "awgn",
            snr_db=snr_3k_db,
            fs=FS,
            seed=500 + trial,
            signal_power=1.0,
            cfo_hz=cfo,
            sro_ppm=float(rng.uniform(-80, 80)),
        )
        y = ch.process(_buffer(modem.data_burst(payload, MODES[0])))
        frames = modem.decode_buffer(y)
        ok += int(len(frames) == 1 and frames[0].payload == payload)
    assert ok >= 5


def test_control_frame_header_is_recognised(modem: Modem, rng: np.random.Generator) -> None:
    payload = rng.integers(0, 256, modem.payload_bytes(), dtype=np.uint8).tobytes()
    y = make_channel("awgn", snr_db=3.0, fs=FS, seed=1, signal_power=1.0).process(
        _buffer(modem.control_burst(payload))
    )
    frames = modem.decode_buffer(y)
    assert len(frames) == 1 and frames[0].frame.sync.header.frame_type is FrameType.CONTROL
    assert frames[0].frame.sync.header_confidence > 1.5
    assert frames[0].payload == payload


def test_no_false_alarms_on_noise_and_voice_like_signals(
    modem: Modem, rng: np.random.Generator
) -> None:
    noise = make_channel("awgn", snr_db=10.0, fs=FS, seed=2, signal_power=1.0).process(
        np.zeros(48000, dtype=complex) + 1e-9
    )
    assert modem.detector.detect(modem.detector.condition(noise), max_frames=3) == []
    t = np.arange(48000) / FS
    voice_like = sum(
        np.exp(2j * np.pi * f * t) * np.exp(-((t - 2.5) ** 2)) for f in (-900, -300, 250, 700)
    )
    assert modem.detector.detect(modem.detector.condition(voice_like), max_frames=3) == []


def test_two_frames_in_one_buffer(modem: Modem, rng: np.random.Generator) -> None:
    p1, p2 = _payload(rng, modem, 4), _payload(rng, modem, 2)
    x = np.concatenate(
        (
            np.zeros(500),
            modem.data_burst(p1, MODES[4]),
            np.zeros(2000),
            modem.data_burst(p2, MODES[2]),
            np.zeros(500),
        )
    )
    y = make_channel("awgn", snr_db=12.0, fs=FS, seed=5, signal_power=1.0).process(x)
    frames = sorted(modem.decode_buffer(y), key=lambda f: f.frame.sync.start)
    assert [f.payload for f in frames] == [p1, p2]


# ── end-to-end decoding ───────────────────────────────────────────────


def test_noiseless_frame_decodes_with_negligible_evm(
    modem: Modem, rng: np.random.Generator
) -> None:
    payload = _payload(rng, modem, 13)  # 64-QAM 5/6 — the least forgiving mode
    frames = modem.decode_buffer(_buffer(modem.data_burst(payload, MODES[13])))
    assert len(frames) == 1 and frames[0].payload == payload
    assert frames[0].frame.snr_carrier_db > 40.0


@pytest.mark.parametrize(
    ("mode_idx", "snr_3k_db"),
    [(0, 0.0), (2, 4.0), (4, 7.0), (8, 12.0), (13, 24.0)],
)
def test_awgn_decoding_with_cfo_and_sro(
    modem: Modem, mode_idx: int, snr_3k_db: float, rng: np.random.Generator
) -> None:
    """Each mode a few dB above its AWGN threshold, with a 123 Hz offset and 80 ppm clock error."""
    ok = 0
    for trial in range(3):
        payload = _payload(rng, modem, mode_idx)
        ch = make_channel(
            "awgn",
            snr_db=snr_3k_db,
            fs=FS,
            seed=100 * mode_idx + trial,
            signal_power=1.0,
            cfo_hz=123.4,
            sro_ppm=80.0,
        )
        y = ch.process(_buffer(modem.data_burst(payload, MODES[mode_idx])))
        frames = modem.decode_buffer(y)
        ok += int(len(frames) == 1 and frames[0].payload == payload)
    assert ok == 3


def test_reported_snr_is_calibrated(modem: Modem, rng: np.random.Generator) -> None:
    payload = _payload(rng, modem, 4)
    y = make_channel("awgn", snr_db=10.0, fs=FS, seed=9, signal_power=1.0).process(
        _buffer(modem.data_burst(payload, MODES[4]))
    )
    f = modem.decode_buffer(y)[0]
    assert f.frame.snr_3k_db == pytest.approx(10.0, abs=1.0)


def test_poor_channel_decoding(modem: Modem, rng: np.random.Generator) -> None:
    """ITU Poor (2 ms / 1 Hz) at +10 dB: BPSK ½ must decode most frames — the pilot grid
    and per-carrier LLR weighting are what make this work."""
    ok = 0
    for trial in range(4):
        payload = _payload(rng, modem, 2)
        ch = make_channel("poor", snr_db=10.0, fs=FS, seed=40 + trial, signal_power=1.0)
        y = ch.process(_buffer(modem.data_burst(payload, MODES[2]), lead=3000))
        frames = modem.decode_buffer(y)
        ok += int(len(frames) == 1 and frames[0].payload == payload)
    assert ok >= 3


def test_interferer_and_impulsive_noise(modem: Modem, rng: np.random.Generator) -> None:
    payload = _payload(rng, modem, 4)
    cfg = ChannelConfig(
        profile="good",
        snr_db=12.0,
        fs=FS,
        signal_power=1.0,
        seed=7,
        impulsive_probability=0.01,
        impulsive_db_above_noise=20.0,
        interferer=InterfererConfig(offset_hz=-2000.0, power_db=-3.0, kind="cw"),
    )
    y = HfChannel(cfg).process(_buffer(modem.data_burst(payload, MODES[4])))
    frames = modem.decode_buffer(y)
    assert len(frames) == 1 and frames[0].payload == payload


def test_full_audio_path_round_trip(modem: Modem, rng: np.random.Generator) -> None:
    """TX → 48 kHz float32 audio → RX converter → detector → decoder."""
    payload = _payload(rng, modem, 8)
    audio = modem.tx.audio(
        np.concatenate((np.zeros(800), modem.data_burst(payload, MODES[8]), np.zeros(800)))
    )
    audio = (audio * 0.3).astype(np.float32)  # typical sound-card level
    y = AudioToBaseband(P).process(audio.astype(np.float64))
    frames = modem.decode_buffer(y)
    assert len(frames) == 1 and frames[0].payload == payload
