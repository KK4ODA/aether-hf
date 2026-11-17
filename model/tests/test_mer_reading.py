"""tools/mer_reading.py: OFDM frames' modulation error ratio against what was sent (issue #15).

A frame through a clean channel must read the same error whichever way it is measured — the
pilots, the data against the re-encoded payload, the data against its nearest points — and the
noise the decoder was given must match the error it got. A frame that did not decode keeps only
the decision-directed figure. A recording made of known frames must be read back frame by frame
through the sidecar's times, as a station's recording would be."""

from __future__ import annotations

import json
import math
import sys
import wave
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import mer_reading as mr  # noqa: E402
from aether_model.channel import make_channel  # noqa: E402
from aether_model.phy.pipeline import Modem  # noqa: E402
from aether_model.waveform import WIDE_2300  # noqa: E402

RUNG = 10
"""A QPSK rung of the wide ladder: decodes on any clean channel used here."""


def _frame(modem: Modem, snr_db: float, rung: int, seed: int) -> tuple[bytes, np.ndarray]:
    rng = np.random.default_rng(seed)
    payload = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
    fs = modem.p.fs_baseband
    x = np.concatenate(
        (
            np.zeros(int(0.4 * fs), complex),
            modem.rung_burst(payload, rung),
            np.zeros(int(0.8 * fs), complex),
        )
    )
    y = make_channel("awgn", snr_db=snr_db, fs=fs, seed=seed, signal_power=1.0).process(x)
    return payload, y


def test_a_clean_frame_reads_the_same_error_every_way() -> None:
    modem = Modem(WIDE_2300)
    payload, y = _frame(modem, 15.0, RUNG, seed=7)
    [dec] = [d for d in modem.decode_buffer(y) if d.ok]
    assert dec.payload is not None and dec.payload[: len(payload)] == payload
    r = mr.measure(modem, dec)
    assert r is not None and r.decoded and r.kind == "data" and r.rung == RUNG
    # the pilots read the noise; the data carriers lose only the channel estimate's own noise
    assert 0.0 <= r.gap_db < 2.0
    # a flat channel: weighting by |H|² changes nothing, and QPSK at 15 dB makes no wrong decision
    assert abs(r.mer_db - r.mer_h_db) < 0.5
    assert abs(r.dd_db - r.mer_db) < 0.2
    # re-estimating the channel from the data costs about 1 dB of its own noise, gains nothing
    assert -2.0 < r.da_db - r.mer_h_db < 0.5
    # the decoder was handed close to the noise it got
    assert abs(r.model_db) < 1.5
    assert len(r.carrier_hz) == len(modem.rx.data_c)
    assert np.all(np.isfinite(r.carrier_mer_h_db))


def test_a_frame_that_did_not_decode_keeps_only_the_decision_directed_figure() -> None:
    modem = Modem(WIDE_2300)
    _, y = _frame(modem, 5.0, 17, seed=3)  # 64-QAM nine decibels under its threshold
    found = [d for d in modem.decode_buffer(y) if d.frame is not None]
    assert found and not found[0].ok
    r = mr.measure(modem, found[0])
    assert r is not None and not r.decoded
    assert math.isnan(r.mer_db) and math.isnan(r.mer_h_db) and math.isnan(r.da_db)
    assert math.isfinite(r.dd_db) and math.isfinite(r.pilot_db)
    assert mr.group_label(r).endswith("failed")


def test_the_data_aided_channel_leaves_each_symbol_out() -> None:
    g = np.arange(12, dtype=np.complex128).reshape(6, 2)
    m = mr.neighbours_mean(g, reach=1)
    # interior: the mean of the symbols either side on the same carrier, never itself
    assert m[2, 0] == (g[1, 0] + g[3, 0]) / 2
    assert m[0, 1] == g[1, 1]
    assert np.allclose(mr.neighbours_mean(np.full((5, 3), 2 + 1j)), 2 + 1j)


def test_a_recording_is_read_back_through_its_sidecar(tmp_path: Path) -> None:
    modem = Modem(WIDE_2300)
    fs = modem.p.fs_baseband
    rng = np.random.default_rng(11)
    pieces = [np.zeros(int(1.0 * fs), complex)]
    frames = []
    t = 1.0
    for rung in (RUNG, 15):
        payload = rng.integers(0, 256, modem.rung_payload_bytes(rung), dtype=np.uint8).tobytes()
        burst = modem.rung_burst(payload, rung)
        frames.append(
            {"t_s": t, "start_s": t, "end_s": t + len(burst) / fs, "decoded": True, "kind": "data"}
        )
        pieces += [burst, np.zeros(int(1.0 * fs), complex)]
        t += (len(burst) + int(1.0 * fs)) / fs
    clean = np.concatenate(pieces)
    y = make_channel("awgn", snr_db=20.0, fs=fs, seed=5, signal_power=1.0).process(clean)
    audio = modem.audio(y, level=0.1)
    wav = tmp_path / "rec.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(48000)
        w.writeframes((np.clip(audio, -1, 1) * 32767).astype(np.int16).tobytes())
    side = tmp_path / "rec.json"
    side.write_text(json.dumps({"session": {"bandwidth_hz": 2300}, "frames": frames}))
    rows = mr.field(wav, side, frames=False, carriers=True)
    assert [r.rung for r in rows] == [RUNG, 15]
    assert all(r.decoded and 0.0 <= r.gap_db < 2.5 for r in rows)
