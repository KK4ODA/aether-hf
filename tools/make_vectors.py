"""Generate the golden test vectors in ``vectors/`` (roadmap P1-9).

    python tools/make_vectors.py [--out vectors]

For every data mode and the control frame: a fixed payload, the 48 kHz audio the model
transmits for it (int16 WAV at −12 dBFS RMS), and a manifest entry with the expected
header, frame position and a SHA-256 of the audio. Two impaired vectors (ITU Poor at
+10 dB with offsets, and pure noise) anchor the receiver.

Vectors are regenerated only when the air interface changes deliberately (ADR); the test
suite fails loudly if the transmitter's output drifts from them, and the Rust core will
be checked against the same files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.channel import make_channel
from aether_model.frame.modes import CONTROL_MODE, LONG, MODES, SHORT
from aether_model.hal.audio import TX_AUDIO_LEVEL, write_wav
from aether_model.phy.pipeline import Modem
from aether_model.waveform import WIDE_2300

LEAD = 2000  # baseband samples of silence before each frame


def payload_for(seed: int, n: int) -> bytes:
    return np.random.default_rng(seed).integers(0, 256, n, dtype=np.uint8).tobytes()


def sha(x: np.ndarray) -> str:
    return hashlib.sha256(np.ascontiguousarray(x).tobytes()).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="vectors")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    modem = Modem(WIDE_2300)
    entries = []

    def emit(name: str, baseband: np.ndarray, meta: dict[str, object]) -> None:
        audio = modem.audio(baseband)
        write_wav(out / f"{name}.wav", WIDE_2300.audio_rate, audio)
        pcm = (np.clip(audio, -1, 1) * 32767).astype("<i2")
        entries.append({"name": name, "file": f"{name}.wav", "audio_sha256": sha(pcm), **meta})

    for mode in MODES:
        payload = payload_for(100 + mode.index, modem.payload_bytes(mode))
        bb = np.concatenate(
            (
                np.zeros(LEAD, dtype=complex),
                modem.data_burst(payload, mode),
                np.zeros(1000, dtype=complex),
            )
        )
        emit(
            f"data_mode{mode.index:02d}_{mode.name.replace('/', '-')}",
            bb,
            {
                "kind": "data",
                "mode": mode.index,
                "payload_hex": payload.hex(),
                "lead_samples_8k": LEAD,
                "frame_samples_8k": LONG.samples,
            },
        )
    payload = payload_for(200, modem.payload_bytes())
    bb = np.concatenate(
        (np.zeros(LEAD, dtype=complex), modem.control_burst(payload), np.zeros(1000, dtype=complex))
    )
    emit(
        "control_frame",
        bb,
        {
            "kind": "control",
            "mode": CONTROL_MODE.index,
            "payload_hex": payload.hex(),
            "lead_samples_8k": LEAD,
            "frame_samples_8k": SHORT.samples,
        },
    )

    # impaired: QPSK 1/2 through ITU Poor at +10 dB with 60 Hz CFO and 40 ppm SRO
    payload = payload_for(300, modem.payload_bytes(MODES[4]))
    bb = np.concatenate(
        (
            np.zeros(LEAD, dtype=complex),
            modem.data_burst(payload, MODES[4]),
            np.zeros(1000, dtype=complex),
        )
    )
    ch = make_channel(
        "poor",
        snr_db=10.0,
        fs=WIDE_2300.fs_baseband,
        seed=31,
        signal_power=1.0,
        cfo_hz=60.0,
        sro_ppm=40.0,
    )
    emit(
        "impaired_poor_10db_qpsk12",
        ch.process(bb),
        {
            "kind": "impaired",
            "mode": 4,
            "payload_hex": payload.hex(),
            "channel": "poor",
            "snr_3k_db": 10.0,
            "cfo_hz": 60.0,
            "sro_ppm": 40.0,
            "lead_samples_8k": LEAD,
        },
    )

    # noise only: must produce no frames
    noise = make_channel(
        "awgn", snr_db=0.0, fs=WIDE_2300.fs_baseband, seed=77, signal_power=1.0
    ).process(np.zeros(40000, dtype=complex) + 1e-9)
    emit("noise_only_5s", noise, {"kind": "noise", "expected_frames": 0})

    manifest = {
        "waveform": WIDE_2300.summary(),
        "tx_audio_level_rms": TX_AUDIO_LEVEL,
        "audio_rate": WIDE_2300.audio_rate,
        "vectors": entries,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n", encoding="utf-8")
    total = sum((out / e["file"]).stat().st_size for e in entries)
    print(f"wrote {len(entries)} vectors to {out} ({total / 1e6:.1f} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
