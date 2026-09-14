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
from numpy.typing import NDArray

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.codec import FrameCodec, coprime_stride
from aether_model.frame.modes import CONTROL_MODE, LONG, MODES, SHORT
from aether_model.phy.constellation import constellation
from aether_model.phy.ofdm import OfdmDemodulator
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.phy.tx import FrameTransmitter
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


def waveform_cases() -> list[dict]:  # type: ignore[type-arg]
    """Whole frames, compared at the carrier level rather than sample by sample.

    Demodulating the transmitter's own output checks the preamble, pilot placement, the mode
    and RV chips, the IFFT scaling and the windowing all at once, in a form small enough to
    commit. A strided set of time samples goes with it so a scaling or windowing error that
    somehow cancelled at the carriers would still show up.

    Peak reduction (ADR-0004) is switched off: the Rust transmitter does not implement it
    yet, and comparing against a model that does would be comparing two different waveforms.
    """
    tx = FrameTransmitter(WIDE_2300, papr_reduction=False)
    dem = OfdmDemodulator(WIDE_2300)
    period = WIDE_2300.symbol_samples
    out = []
    rng = np.random.default_rng(31337)
    cases = [
        (MODES[0], LONG, FrameType.DATA, 0),
        (MODES[4], LONG, FrameType.DATA, 1),
        (MODES[13], LONG, FrameType.DATA, 3),
        (CONTROL_MODE, SHORT, FrameType.CONTROL, 0),
    ]
    for mode, layout, frame_type, rv in cases:
        codec = FrameCodec(mode, layout)
        payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
        qam = codec.encode(payload, rv)
        header = (
            FrameHeader(FrameType.DATA, mode.index, rv)
            if frame_type is FrameType.DATA
            else FrameHeader(FrameType.CONTROL)
        )
        waveform = tx.baseband(header, layout, qam)
        # carrier values of every symbol but the last (which has no successor to overlap)
        carriers = [
            complex_list(dem.carriers(waveform, i * period))
            for i in range(layout.total_symbols - 1)
        ]
        power = float(np.mean(np.abs(waveform) ** 2))
        peak = float(np.max(np.abs(waveform) ** 2))
        out.append(
            {
                "mode": mode.index,
                "mode_name": mode.name,
                "layout": layout.name,
                "frame_type": frame_type.name,
                "rv": rv,
                "payload": payload.hex(),
                "n_samples": len(waveform),
                "mean_power": power,
                "papr_db": float(10 * np.log10(peak / power)),
                "stride": 97,
                "strided_samples": complex_list(waveform[::97]),
                "carriers": carriers,
            }
        )
    return out


def passband_input(n: int) -> NDArray[np.complex128]:
    """A deterministic multi-tone test signal, defined so both languages build it identically.

    No RNG: a vector file that recorded only every k-th sample could not tell the Rust side
    what the other samples were, and a filter's output at one sample depends on many."""
    k = np.arange(n)
    real = np.cos(2 * np.pi * 0.037 * k) + 0.5 * np.cos(2 * np.pi * 0.011 * k + 0.7)
    imag = np.sin(2 * np.pi * 0.023 * k) - 0.3 * np.sin(2 * np.pi * 0.005 * k)
    return 0.3 * (real + 1j * imag)


def passband_case() -> dict[str, object]:
    """Filter taps and a round trip through the audio front end.

    The taps are a design, not a measurement: two implementations of the same window method
    either agree to the last bit or one of them is wrong, so they are compared in full."""
    from aether_model.phy.passband import (
        AudioToBaseband,
        BasebandToAudio,
        band_limit_taps,
        interpolation_taps,
    )

    n = 2048
    baseband = passband_input(n)
    audio = BasebandToAudio(WIDE_2300).process(baseband)
    back = AudioToBaseband(WIDE_2300).process(audio)
    return {
        "band_limit_taps": [float(x) for x in band_limit_taps(WIDE_2300)],
        "resample_taps": [float(x) for x in interpolation_taps(WIDE_2300)],
        "tx_delay_samples": BasebandToAudio(WIDE_2300).tx_delay_samples,
        "rx_delay_samples": AudioToBaseband(WIDE_2300).rx_delay_samples,
        "n_samples": n,
        "stride": 37,
        "audio_out": [float(x) for x in audio[::37]],
        "baseband_back": complex_list(back[::37]),
    }


def blanker_input(n: int) -> NDArray[np.complex128]:
    """A deterministic test signal for the blanker: silence, then a burst with peaks, with
    impulses of known size dropped into both.

    Closed form rather than random, so the Rust side can build the same samples. The burst
    has a peaky envelope on purpose — the blanker's whole job is to tell a peak that belongs
    to the signal from one that does not."""
    k = np.arange(n)
    envelope = np.where(k < n // 4, 0.02, 1.0)  # an abrupt onset, as a burst on a quiet band
    phase = 2 * np.pi * (0.031 * k + 0.7 * np.sin(2 * np.pi * 0.0013 * k))
    peaks = 1.0 + 0.8 * np.cos(2 * np.pi * 0.0071 * k) + 0.5 * np.cos(2 * np.pi * 0.017 * k)
    x = envelope * peaks * np.exp(1j * phase)
    for index, size in ((n // 8, 30.0), (n // 2, 50.0), (3 * n // 4, 8.0)):
        x[index] = size * np.exp(1j * index)
    return np.asarray(x, dtype=np.complex128)


def blanker_case() -> dict[str, object]:
    """The impulse blanker, offline and streaming.

    The blanker decides which samples to throw away, so what matters is *which* — an
    implementation that blanks a different set is a different receiver. The blanked indices
    are therefore compared exactly, and the envelope estimate behind them to a tolerance."""
    from aether_model.phy.blanker import NoiseBlanker, StreamingBlanker

    n = 8192
    x = blanker_input(n)
    blanker = NoiseBlanker()
    result = blanker.process(x)

    streamed: dict[str, object] = {}
    for block in (683, 4096):
        sb = StreamingBlanker()
        pieces = [sb.process(x[i : i + block]) for i in range(0, n, block)]
        out = np.concatenate([*pieces, sb.flush()])
        streamed[str(block)] = {
            "latency_samples": sb.latency_samples,
            "blanked": [int(i) for i in np.flatnonzero(out == 0)],
        }

    return {
        "threshold_sigma": blanker.threshold_sigma,
        "window": blanker.window,
        "segment_span": blanker.segment_span,
        "robust_span_samples": blanker.robust_span_samples,
        "n_samples": n,
        "stride": 53,
        "envelope_rms": [float(v) for v in blanker.envelope_rms(x)[::53]],
        "blanked": [int(i) for i in np.flatnonzero(result.blanked)],
        "streaming": streamed,
    }


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
        "waveform_frames": waveform_cases(),
        "passband": passband_case(),
        "blanker": blanker_case(),
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    counts = {k: len(v) for k, v in document.items() if isinstance(v, list)}
    print(f"wrote {out} ({counts})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
