"""The other station's frames as a witness of this station's receive passband (the receive-filter
review, Gate 1).

    python -I tools/passband_witness.py RECORDING.wav SIDECAR.json [WAV SIDECAR ...]

Every frame the recording station decoded is decoded again by the model. An OFDM frame's channel
estimate gives the response at each of its carriers (|H|² averaged over the frame's symbols); a
tone frame, its payload once decoded saying which tone went in each symbol, gives the energy at
each tone it sent. Each frame's levels are taken relative to its own median carrier or tone, so
the sender's level and the path's loss drop out, and what is left is the shape across frequency:
the sending radio's transmit filter, the path's selective fading and this radio's receive
response together. A filter stays from frame to frame and fading moves, so the median over frames
is printed with how many frames stand behind each frequency.
"""

from __future__ import annotations

import json
import sys
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.codec import tone_codec
from aether_model.phy import tone as T
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import DecodedFrame, Modem
from aether_model.waveform import NARROW_500, WIDE_2300, WaveformParams

MARGIN_S = 0.3
"""How far either side of a recorded frame's span the slice reaches: the front end's delay and
the recorded timing's rounding."""
BAND_HZ = 100
"""The width the levels are gathered into for printing."""


def read(wav: Path) -> np.ndarray:
    with wave.open(str(wav)) as w:
        raw = w.readframes(w.getnframes())
        x = np.frombuffer(raw, dtype=np.int16).astype(np.float64) / 32768
        channels = w.getnchannels()
    return x.reshape(-1, channels)[:, 0] if channels > 1 else x


def ofdm_levels(dec: DecodedFrame, params: WaveformParams) -> dict[float, float]:
    """An OFDM frame's response per carrier, dB relative to its median carrier."""
    assert dec.frame is not None
    h2 = np.mean(np.abs(dec.frame.channel) ** 2, axis=0)
    n = params.n_carriers
    hz = params.centre_hz + (np.arange(n) - n // 2) * params.subcarrier_spacing_hz
    db = 10 * np.log10(h2 / np.median(h2))
    return dict(zip(hz.tolist(), db.tolist(), strict=True))


def tone_levels(dec: DecodedFrame, y: np.ndarray, params: WaveformParams) -> dict[float, float]:
    """A tone frame's energy at each tone it sent, dB relative to its median tone."""
    sync = dec.tone_sync
    assert sync is not None and dec.payload is not None
    kind = sync.kind
    data = tone_codec(kind).encode(dec.payload[: kind.payload_bytes], sync.rv)
    if kind.speed == 1:
        sent, _ = T.frame_symbols(kind, data, sync.rv, sync.middle)
        e = T.tone_energies(y, sync.start, sync.cfo_hz, len(sent), kind.num)
        num = kind.num
    else:
        sent = data
        e = T.data_energies(y, kind, sync.start, sync.cfo_hz)
        num = kind.data
    sent = np.asarray(sent[: len(e)])
    level = e[np.arange(len(sent)), sent]
    ref = float(np.median(level))
    out = {}
    for t in range(num.tones):
        mine = level[sent == t]
        if len(mine):
            hz = float(params.centre_hz + num.tone_hz(t))
            out[hz] = 10 * np.log10(float(np.mean(mine)) / ref)
    return out


def witness(wav: Path, side: Path) -> None:
    d = json.loads(side.read_text(encoding="utf-8"))
    hz_air = int(d["session"].get("bandwidth_hz") or 2300)
    params = WIDE_2300 if hz_air == 2300 else NARROW_500
    bb = AudioToBaseband(params).process(read(wav))
    fs = params.fs_baseband
    modem = Modem(params)
    by_band: dict[str, dict[int, list[float]]] = {"ofdm": {}, "tone": {}}
    frames = {"ofdm": 0, "tone": 0}
    for fr in d["frames"]:
        if not fr.get("decoded") or fr.get("start_s") is None:
            continue
        a = max(0, int((fr["start_s"] - MARGIN_S) * fs))
        b = min(len(bb), int((fr["end_s"] + MARGIN_S) * fs))
        x = bb[a:b]
        if modem.blanker is not None:
            x = modem.blanker.process(x).samples
        y = modem.detector.condition(x)
        for dec in modem.decode_buffer(bb[a:b], max_frames=2):
            if not dec.ok:
                continue
            try:
                family = "ofdm" if dec.frame is not None else "tone"
                levels = (
                    ofdm_levels(dec, params) if family == "ofdm" else tone_levels(dec, y, params)
                )
            except (ValueError, IndexError):
                break
            frames[family] += 1
            per_band: dict[int, list[float]] = {}
            for f, db in levels.items():
                per_band.setdefault(int(f // BAND_HZ * BAND_HZ), []).append(db)
            for band, values in per_band.items():
                by_band[family].setdefault(band, []).append(float(np.mean(values)))
            break
    print(f"{wav.name} ({hz_air} Hz air)")
    for family, bands in by_band.items():
        if not bands:
            print(f"  {family}: no frame decoded")
            continue
        print(f"  {family}: {frames[family]} frames; dB under each frame's median, by {BAND_HZ} Hz")
        print(
            "    "
            + "  ".join(
                f"{band}:{np.median(v):+.1f}({len(v)})" for band, v in sorted(bands.items())
            )
        )


def main() -> None:
    args = sys.argv[1:]
    if not args or len(args) % 2:
        raise SystemExit(__doc__)
    for wav, side in zip(args[::2], args[1::2], strict=True):
        witness(Path(wav), Path(side))


if __name__ == "__main__":
    main()
