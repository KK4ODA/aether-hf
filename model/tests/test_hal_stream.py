"""P1-8: streaming receiver, WAV/simulator audio backends, level meter."""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest

from aether_model.channel import ChannelConfig, make_channel
from aether_model.frame.modes import MODES
from aether_model.hal.audio import (
    TX_AUDIO_LEVEL,
    LevelMeter,
    SimulatorBackend,
    WavFileBackend,
    read_wav,
    write_wav,
)
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import Modem
from aether_model.phy.stream import StreamingReceiver
from aether_model.waveform import WIDE_2300

P = WIDE_2300
FS = P.fs_baseband


@pytest.fixture(scope="module")
def modem() -> Modem:
    return Modem(P)


def _three_frames(modem: Modem, rng: np.random.Generator) -> tuple[list[bytes], np.ndarray]:
    payloads, parts = [], [np.zeros(3000, dtype=complex)]
    for idx in (4, 0, 8):
        p = rng.integers(0, 256, modem.payload_bytes(MODES[idx]), dtype=np.uint8).tobytes()
        payloads.append(p)
        parts += [
            modem.data_burst(p, MODES[idx]),
            np.zeros(int(rng.integers(1500, 4000)), dtype=complex),
        ]
    return payloads, np.concatenate(parts)


# ── streaming receiver ────────────────────────────────────────────────


@pytest.mark.parametrize("block", [1024, 2048, 7777])
def test_streaming_matches_offline_for_any_block_size(
    modem: Modem, block: int, rng: np.random.Generator
) -> None:
    payloads, x = _three_frames(modem, rng)
    y = make_channel(
        "moderate", snr_db=12.0, fs=FS, seed=3, signal_power=1.0, cfo_hz=-60.0
    ).process(x)
    offline = [f.payload for f in modem.decode_buffer(y, max_frames=5)]
    rx = StreamingReceiver(P)
    streamed = []
    for i in range(0, len(y), block):
        streamed += [f.payload for f in rx.feed(y[i : i + block])]
    streamed += [f.payload for f in rx.feed(np.zeros(4000, dtype=complex))]  # flush
    assert offline == payloads
    assert streamed == payloads
    assert rx.frames_decoded == 3


def test_streaming_reports_absolute_positions(modem: Modem, rng: np.random.Generator) -> None:
    _, x = _three_frames(modem, rng)
    rx = StreamingReceiver(P, max_buffer_s=3.0)
    starts = []
    for i in range(0, len(x), 4000):
        starts += [f.frame.sync.start for f in rx.feed(x[i : i + 4000])]
    starts += [f.frame.sync.start for f in rx.feed(np.zeros(4000, dtype=complex))]
    assert len(starts) == 3
    assert starts[0] == 3000 + rx.delay_samples
    assert starts == sorted(starts) and len(set(starts)) == 3


# ── WAV backend ───────────────────────────────────────────────────────


def test_wav_round_trip_through_files(
    modem: Modem, rng: np.random.Generator, tmp_path: Path
) -> None:
    payloads, x = _three_frames(modem, rng)
    audio = modem.audio(x)
    assert abs(20 * np.log10(np.sqrt(np.mean(audio[24000:30000] ** 2)) / TX_AUDIO_LEVEL)) < 3.0
    path = tmp_path / "tx.wav"
    write_wav(path, P.audio_rate, audio)
    rate, back = read_wav(path)
    assert rate == P.audio_rate and len(back) == len(audio)
    backend = WavFileBackend(path, tmp_path / "out.wav", rate=P.audio_rate)
    backend.start()
    conv = AudioToBaseband(P)
    rx = StreamingReceiver(P)
    got: list[bytes | None] = []
    while not backend.exhausted:
        got += [f.payload for f in rx.feed(conv.process(backend.read(4096).astype(np.float64)))]
    got += [f.payload for f in rx.feed(np.zeros(4000, dtype=complex))]
    backend.write(audio[:1000])
    backend.stop()
    assert got == payloads
    assert backend.meter.peak < 1.0 and backend.meter.clipped == 0
    assert (tmp_path / "out.wav").exists()


def test_write_wav_clips_instead_of_wrapping(tmp_path: Path) -> None:
    path = tmp_path / "clip.wav"
    write_wav(path, 48000, np.array([0.0, 2.0, -3.0], dtype=np.float32))
    _, x = read_wav(path)
    np.testing.assert_allclose(x, [0.0, 32767 / 32768, -32767 / 32768], atol=1e-6)


# ── simulator backend ─────────────────────────────────────────────────


def test_simulator_backend_loopback_decodes(modem: Modem, rng: np.random.Generator) -> None:
    payload = rng.integers(0, 256, modem.payload_bytes(MODES[4]), dtype=np.uint8).tobytes()
    be = SimulatorBackend(ChannelConfig(profile="good", snr_db=12.0, seed=5))
    be.start()
    be.write(
        modem.audio(
            np.concatenate(
                (
                    np.zeros(800, dtype=complex),
                    modem.data_burst(payload, MODES[4]),
                    np.zeros(800, dtype=complex),
                )
            )
        )
    )
    conv = AudioToBaseband(P)
    rx = StreamingReceiver(P)
    got = []
    for _ in range(40):
        got += [f.payload for f in rx.feed(conv.process(be.read(2048).astype(np.float64)))]
    assert got == [payload]
    assert be.meter.rms > 0.0


def test_simulator_backend_cross_connected_peers(modem: Modem, rng: np.random.Generator) -> None:
    a = SimulatorBackend(ChannelConfig(profile="awgn", snr_db=15.0, seed=1))
    b = SimulatorBackend(ChannelConfig(profile="awgn", snr_db=15.0, seed=2))
    a.peer, b.peer = b, a
    payload = rng.integers(0, 256, modem.payload_bytes(MODES[2]), dtype=np.uint8).tobytes()
    a.write(
        modem.audio(
            np.concatenate((np.zeros(500, dtype=complex), modem.data_burst(payload, MODES[2])))
        )
    )
    conv_b, rx_b = AudioToBaseband(P), StreamingReceiver(P)
    got_b = []
    for _ in range(40):
        got_b += [f.payload for f in rx_b.feed(conv_b.process(b.read(2048).astype(np.float64)))]
    assert got_b == [payload]
    # only the noise floor arrives at a's own receiver (15 dB below the TX level)
    own = a.read(4096).astype(np.float64)
    assert np.sqrt(np.mean(own**2)) < TX_AUDIO_LEVEL / 3


def test_simulator_silence_carries_the_noise_floor() -> None:
    be = SimulatorBackend(ChannelConfig(profile="awgn", snr_db=0.0, seed=9))
    block = be.read(4800)
    assert len(block) == 4800
    assert 0.0 < np.sqrt(np.mean(block.astype(np.float64) ** 2)) < TX_AUDIO_LEVEL


# ── level meter ───────────────────────────────────────────────────────


def test_level_meter_tracks_peak_rms_and_clipping() -> None:
    m = LevelMeter(window_s=0.5, rate=48000)
    m.update(0.5 * np.ones(1000, dtype=np.float32))
    assert m.peak == pytest.approx(0.5) and m.rms == pytest.approx(0.5) and m.clipped == 0
    m.update(np.array([1.0, -1.0, 0.0], dtype=np.float32))
    assert m.clipped == 2 and m.peak == 1.0
    assert m.peak_dbfs == pytest.approx(0.0)


def test_sounddevice_backend_enumerates_when_available() -> None:
    pytest.importorskip("sounddevice")
    from aether_model.hal.audio import SoundDeviceBackend

    devices = SoundDeviceBackend.list_devices()
    assert isinstance(devices, list)
