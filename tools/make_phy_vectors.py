"""Generate PHY cross-validation vectors for the Rust core (roadmap P3-2).

    python tools/make_phy_vectors.py [--out core/aether-phy/tests/data/phy_vectors.json]

ADR-0001 requires the core to agree with the model. What "agree" means differs by layer, and
this file separates the two so the Rust tests can assert the right thing about each:

* **exact** — the mode table, frame layouts, constellation points and bit labels, the
  interleaver permutation, and the coded symbols a payload maps to. These are integer or
  exactly-representable quantities and any difference is a bug.
* **approximate** — LLR values, which are floating-point arithmetic over the same formula.
  Two correct implementations may differ in the last bits, so the Rust side asserts a
  tolerance rather than equality, and the tolerance it uses is recorded here.

Complex values are written as `[re, im]` pairs with full `repr` precision.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.codec import FrameCodec, coprime_stride
from aether_model.frame.modes import CONTROL_MODE, LONG, MODES, SHORT
from aether_model.phy.constellation import constellation
from aether_model.waveform import WIDE_2300, Modulation

MODULATIONS = [
    Modulation.BPSK,
    Modulation.QPSK,
    Modulation.PSK8,
    Modulation.QAM16,
    Modulation.QAM64,
]


def complex_list(values: np.ndarray) -> list[list[float]]:
    return [[float(v.real), float(v.imag)] for v in np.asarray(values)]


def pack(bits: np.ndarray) -> str:
    """Bits, MSB-first, as hex. Storing the *bits* a codec produces rather than the complex
    symbols keeps this file small and pins the quantity that has to be exact: mapping bits to
    points is a separate, separately tested step."""
    return np.packbits(np.asarray(bits, dtype=np.uint8)).tobytes().hex()


def waveform_case() -> dict:  # type: ignore[type-arg]
    p = WIDE_2300
    return {
        "fs_baseband": p.fs_baseband,
        "fft_size": p.fft_size,
        "cp_samples": p.cp_samples,
        "taper_samples": p.taper_samples,
        "centre_hz": p.centre_hz,
        "audio_rate": p.audio_rate,
        "subcarrier_spacing_hz": p.subcarrier_spacing_hz,
        "useful_symbol_s": p.useful_symbol_s,
        "symbol_samples": p.symbol_samples,
        "symbol_period_s": p.symbol_period_s,
        "symbol_rate_bd": p.symbol_rate_bd,
        "effective_cp_s": p.effective_cp_s,
        "n_carriers": p.n_carriers,
        "n_pilot_carriers": p.n_pilot_carriers,
        "n_data_carriers": p.n_data_carriers,
        "occupied_bandwidth_hz": p.occupied_bandwidth_hz,
        "resample_factor": p.resample_factor,
    }


def layout_cases() -> list[dict]:  # type: ignore[type-arg]
    return [
        {
            "name": layout.name,
            "data_symbols": layout.data_symbols,
            "total_symbols": layout.total_symbols,
            "duration_s": layout.duration_s,
            "samples": layout.samples,
            "qam_symbols": layout.qam_symbols,
            "pilot_symbol_indices": list(layout.pilot_symbol_indices),
            "n_payload_symbols": layout.n_payload_symbols,
        }
        for layout in (LONG, SHORT)
    ]


def mode_cases() -> list[dict]:  # type: ignore[type-arg]
    return [
        {
            "index": m.index,
            "name": m.name,
            "bits_per_symbol": m.modulation.bits_per_symbol,
            "rate_num": m.code_rate.numerator,
            "rate_den": m.code_rate.denominator,
            "coded_bits": m.coded_bits(LONG),
            "info_bits": m.info_bits(LONG),
            "payload_bytes": m.payload_bytes(LONG),
            "base_graph": m.base_graph(LONG),
            "lifting_size": m.lifting_size(LONG),
            "net_bit_rate": m.net_bit_rate(LONG),
        }
        for m in MODES
    ]


def constellation_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    rng = np.random.default_rng(20260913)
    for mod in MODULATIONS:
        c = constellation(mod)
        m = mod.bits_per_symbol
        bits = rng.integers(0, 2, m * 48).astype(np.uint8)
        symbols = c.map(bits)
        noise = 0.35
        out.append(
            {
                "modulation": mod.name,
                "bits_per_symbol": m,
                "points": complex_list(c.points),
                "min_distance": float(c.min_distance),
                "bits": [int(b) for b in bits],
                "symbols": complex_list(symbols),
                "noise_var": noise,
                "llr": [float(v) for v in c.llr(symbols, noise)],
            }
        )
    return out


def interleaver_cases() -> list[dict]:  # type: ignore[type-arg]
    out = []
    for e in sorted({m.coded_bits(LONG) for m in MODES} | {CONTROL_MODE.coded_bits(SHORT)}):
        stride = coprime_stride(e)
        out.append(
            {
                "e": e,
                "stride": stride,
                "permutation_head": [(k * stride) % e for k in range(24)],
            }
        )
    return out


def codec_cases() -> list[dict]:  # type: ignore[type-arg]
    """Encoded symbols for every mode and redundancy version — the strongest exact check,
    since it exercises CRC, LDPC, rate matching, interleaving and mapping together."""
    out = []
    rng = np.random.default_rng(4242)
    for m in MODES:
        codec = FrameCodec(m, LONG)
        mapper = constellation(m.modulation)
        payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
        for rv in range(4):
            symbols = codec.encode(payload, rv)
            out.append(
                {
                    "mode": m.index,
                    "mode_name": m.name,
                    "layout": "long",
                    "rv": rv,
                    "payload": payload.hex(),
                    "n_symbols": len(symbols),
                    "coded_bits": int(m.coded_bits(LONG)),
                    "interleaved_bits": pack(mapper.hard(symbols)),
                    "symbols_head": complex_list(symbols[:8]),
                }
            )
    control = FrameCodec(CONTROL_MODE, SHORT)
    mapper = constellation(CONTROL_MODE.modulation)
    payload = bytes(range(control.payload_bytes))
    symbols = control.encode(payload, 0)
    out.append(
        {
            "mode": CONTROL_MODE.index,
            "mode_name": CONTROL_MODE.name,
            "layout": "short",
            "rv": 0,
            "payload": payload.hex(),
            "n_symbols": len(symbols),
            "coded_bits": int(CONTROL_MODE.coded_bits(SHORT)),
            "interleaved_bits": pack(mapper.hard(symbols)),
            "symbols_head": complex_list(symbols[:8]),
        }
    )
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-phy/tests/data/phy_vectors.json")
    args = ap.parse_args()

    document = {
        "note": "Generated by tools/make_phy_vectors.py from the Python reference model. "
        "Integer-valued fields are exact; LLRs are floating point and are compared to a "
        "tolerance by the Rust side.",
        "llr_tolerance": 1e-9,
        "symbol_tolerance": 1e-12,
        "waveform": waveform_case(),
        "layouts": layout_cases(),
        "modes": mode_cases(),
        "constellations": constellation_cases(),
        "interleaver": interleaver_cases(),
        "codec": codec_cases(),
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    counts = {k: len(v) for k, v in document.items() if isinstance(v, list)}
    print(f"wrote {out} ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
