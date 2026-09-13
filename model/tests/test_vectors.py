"""P1-9: golden vectors — the transmitter must reproduce them bit-exactly and the receiver
must decode them. Regenerate with ``tools/make_vectors.py`` only for a deliberate air-
interface change (record it in an ADR)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np
import pytest

from aether_model.frame.modes import MODES
from aether_model.hal.audio import read_wav
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300

VECTORS = Path(__file__).resolve().parents[2] / "vectors"
MANIFEST = VECTORS / "manifest.json"

pytestmark = pytest.mark.skipif(not MANIFEST.exists(), reason="vectors/ not generated")


def _manifest() -> dict:  # type: ignore[type-arg]
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def _entries(kind: str) -> list[dict]:  # type: ignore[type-arg]
    return [e for e in _manifest()["vectors"] if e["kind"] == kind]


@pytest.fixture(scope="module")
def modem() -> Modem:
    return Modem(WIDE_2300)


def test_manifest_matches_current_waveform() -> None:
    m = _manifest()
    assert m["waveform"] == WIDE_2300.summary()
    assert m["audio_rate"] == WIDE_2300.audio_rate
    assert len(_entries("data")) == len(MODES)


@pytest.mark.parametrize("entry", _entries("data") + _entries("control"), ids=lambda e: e["name"])
def test_transmitter_reproduces_vector_bit_exactly(entry: dict, modem: Modem) -> None:  # type: ignore[type-arg]
    payload = bytes.fromhex(entry["payload_hex"])
    lead = np.zeros(entry["lead_samples_8k"], dtype=complex)
    tail = np.zeros(1000, dtype=complex)
    if entry["kind"] == "data":
        bb = np.concatenate((lead, modem.data_burst(payload, MODES[entry["mode"]]), tail))
    else:
        bb = np.concatenate((lead, modem.control_burst(payload), tail))
    audio = modem.audio(bb)
    pcm = (np.clip(audio, -1, 1) * 32767).astype("<i2")
    assert hashlib.sha256(pcm.tobytes()).hexdigest() == entry["audio_sha256"]


@pytest.mark.parametrize(
    "entry", _entries("data") + _entries("control") + _entries("impaired"), ids=lambda e: e["name"]
)
def test_receiver_decodes_vector(entry: dict, modem: Modem) -> None:  # type: ignore[type-arg]
    rate, audio = read_wav(VECTORS / entry["file"])
    assert rate == WIDE_2300.audio_rate
    y = AudioToBaseband(WIDE_2300).process(audio.astype(np.float64))
    frames = modem.decode_buffer(y, max_frames=2)
    assert len(frames) == 1
    assert frames[0].payload == bytes.fromhex(entry["payload_hex"])
    assert frames[0].frame.mode == entry["mode"]


def test_noise_vector_yields_no_frames(modem: Modem) -> None:
    (entry,) = _entries("noise")
    _, audio = read_wav(VECTORS / entry["file"])
    y = AudioToBaseband(WIDE_2300).process(audio.astype(np.float64))
    assert modem.decode_buffer(y, max_frames=3) == []
