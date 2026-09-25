"""Measure the spectrum every Aether transmission occupies, for the regulatory policy.

`core/aetherd/data/occupancy.json` is what the daemon's regulatory gate (ADR-0018) reads to
turn a dial frequency into the RF range a transmission covers. It is measured here, from the
model's transmitter — the specification; the port is held to it bit for bit by the vector
files — and not typed by hand, because compliance depends on what the waveform actually
occupies, not on its name: the "500 Hz" air's OFDM rungs measure 560–710 Hz.

Every rung of both airs and both control frames is sent as a run of frames with random
payloads through the transmit chain the daemon uses (``BasebandToAudio``: the band-limit
filter, up-conversion to the audio centre, 48 kHz), and its averaged spectrum is read two
ways, because 47 CFR §97.3(a)(8) — "the width of a frequency band outside of which the mean
power of the transmitted signal is attenuated at least 26 dB below the mean power of the
transmitted signal within the band" — can be read two ways:

* ``power``: the band holding all but 1/398 of the power (26 dB), 0.125 % left out on each
  side — the reading that compares two powers, as the words do;
* ``spectral``: the band outside which the spectral density stays 26 dB below the mean
  density within it — the reading an analyser's "−26 dB bandwidth" takes. It is the wider
  of the two for every Aether emission.

Both are kept; the regulatory profile says which one decides (the United States profile
takes the wider of the two, ADR-0018). Edges are offsets from the audio centre in hertz, at
the resolution of the spectrum (48 000 / 8 192 ≈ 5.9 Hz).

Usage::

    python tools/make_occupancy.py            # writes core/aetherd/data/occupancy.json
    python tools/make_occupancy.py --out FILE
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np
from scipy import signal

from aether_model.phy.passband import BasebandToAudio
from aether_model.phy.pipeline import Modem
from aether_model.waveform import NARROW_500, WIDE_2300, WaveformParams

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "core" / "aetherd" / "data" / "occupancy.json"

FS = 48_000
NPERSEG = 8192
"""Welch segment: 5.9 Hz bins with a Hann window — finer than any edge that matters here."""
FRAMES = 8
"""Frames per measurement: enough random payloads that the spectrum is the waveform's, not one
frame's."""
OUTSIDE_DB = 26.0
"""§97.3(a)(8)."""


def _edges(audio: np.ndarray, centre: float) -> dict[str, list[float]]:
    f, p = signal.welch(audio, fs=FS, nperseg=NPERSEG, window="hann", scaling="density")
    total = float(p.sum())
    tail = 10 ** (-OUTSIDE_DB / 10) / (1 + 10 ** (-OUTSIDE_DB / 10)) / 2
    c = np.cumsum(p) / total
    power = (float(f[np.searchsorted(c, tail)]), float(f[np.searchsorted(c, 1 - tail)]))
    # the spectral reading, against the mean density within the band itself: widen until
    # the band stops moving
    lo, hi = power
    for _ in range(50):
        band = (f >= lo) & (f <= hi)
        mean = float(p[band].mean())
        above = np.nonzero(p >= mean * 10 ** (-OUTSIDE_DB / 10))[0]
        nlo, nhi = float(f[above[0]]), float(f[above[-1]])
        if (nlo, nhi) == (lo, hi):
            break
        lo, hi = nlo, nhi
    return {
        "power": [round(power[0] - centre, 3), round(power[1] - centre, 3)],
        "spectral": [round(lo - centre, 3), round(hi - centre, 3)],
    }


def _measure(bursts: list[np.ndarray], params: WaveformParams) -> dict[str, list[float]]:
    tx = BasebandToAudio(params)
    audio = np.concatenate([tx.process(b) for b in bursts] + [tx.process(np.zeros(4096, complex))])
    return _edges(audio, params.centre_hz)


def _air(params: WaveformParams, seed: int) -> dict[str, Any]:
    rng = np.random.default_rng(seed)
    modem = Modem(params, blank_impulses=False)

    def payload(n: int) -> bytes:
        return rng.integers(0, 256, n, dtype=np.uint8).tobytes()

    rungs = []
    for index, rung in enumerate(modem.air.ladder):
        size = modem.rung_payload_bytes(index)
        bursts = [modem.rung_burst(payload(size), index, rv % 4) for rv in range(FRAMES)]
        tone = rung.tone
        rungs.append(
            {
                "rung": index,
                "name": tone.name if tone is not None else rung.mode.name,  # type: ignore[union-attr]
                "family": "tone" if tone is not None else "ofdm",
                **_measure(bursts, params),
            }
        )
    control = {
        family: _measure([modem.control_burst(payload(7), 0, floor) for _ in range(FRAMES)], params)
        for family, floor in (("ordinary", False), ("floor", True))
    }
    return {
        "centre_hz": params.centre_hz,
        "nominal_hz": params.occupied_bandwidth_hz,
        "rungs": rungs,
        "control": control,
    }


def build() -> dict[str, Any]:
    return {
        "generator": "tools/make_occupancy.py",
        "definition": "47 CFR 97.3(a)(8): 26 dB",
        "resolution_hz": FS / NPERSEG,
        "frames": FRAMES,
        "airs": {"2300": _air(WIDE_2300, 1), "500": _air(NARROW_500, 2)},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--out", type=Path, default=OUT)
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(build(), indent=1) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
