"""Generate the numeric tables inside the air-interface specification (roadmap P2-7).

    python tools/make_spec.py [--check]

``docs/spec/air-interface.md`` is a public document, and a public document that disagrees
with the implementation is worse than none at all. Every number in it that the code also
knows — waveform numerology, the mode table, frame layouts, the chip and preamble constants
— is generated from the model and written between ``<!-- BEGIN:name -->`` markers. Prose
outside the markers is written by hand and never touched.

``--check`` regenerates into memory and fails if the file is out of date, so CI catches a
spec that has drifted from the code instead of a reader catching it on the air.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import (
    LONG,
    NARROW,
    PAYLOAD_CRC,
    PREAMBLE_SYMBOLS,
    SYNC_PATTERNS,
    SYNC_SYMBOLS,
    TONE_CONTROL,
    TONE_DATA,
    TONE_FAST,
    TONE_NUMEROLOGY,
    WIDE,
    AirInterface,
    FrameLayout,
)
from aether_model.link.frames import CALL_BYTES, CONTROL_BYTES, DATA_HEADER, WINDOW
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    NARROW_AWGN_THRESHOLD_DB,
    TONE_CONTROL_THRESHOLD_DB,
)
from aether_model.phy import tone
from aether_model.phy.papr import CLIP_TARGET_DB, CLIP_TARGET_DENSE_DB
from aether_model.phy.preamble import (
    MODE_CHIP_SEED,
    N_RV,
    SC_SEEDS,
    preamble,
)

SPEC = Path(__file__).resolve().parents[1] / "docs" / "spec" / "air-interface.md"


def _table(headers: list[str], rows: list[list[str]]) -> str:
    out = ["| " + " | ".join(headers) + " |", "|" + "---|" * len(headers)]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def waveform_block(air: AirInterface = WIDE) -> str:
    P = air.params
    pre = preamble(P)
    rows = [
        ["Baseband sample rate", f"{P.fs_baseband:.0f} Hz", "complex"],
        ["Audio sample rate", f"{P.audio_rate:.0f} Hz", f"interpolation x{P.resample_factor}"],
        ["FFT size N", f"{P.fft_size}", "subcarriers in the transform"],
        ["Subcarrier spacing", f"{P.subcarrier_spacing_hz:.0f} Hz", "fs / N"],
        ["Useful symbol time", f"{P.useful_symbol_s * 1e3:.1f} ms", "1 / spacing"],
        ["Cyclic prefix", f"{P.cp_samples} samples ({P.cp_s * 1e3:.1f} ms)", "before windowing"],
        [
            "Window taper",
            f"{P.taper_samples} samples",
            f"raised cosine; effective CP {P.effective_cp_s * 1e3:.1f} ms",
        ],
        [
            "Symbol period",
            f"{P.symbol_samples} samples ({P.symbol_period_s * 1e3:.2f} ms)",
            "CP + N",
        ],
        ["Symbol rate", f"{P.symbol_rate_bd:.2f} Bd", ""],
        ["Active subcarriers", f"{P.n_carriers}", "centred on the passband centre"],
        ["Comb pilots", f"{P.n_pilot_carriers}", f"every {P.pilot_carrier_spacing}th, both edges"],
        ["Data subcarriers", f"{P.n_data_carriers}", "per ordinary symbol"],
        ["Occupied bandwidth", f"{P.occupied_bandwidth_hz:.0f} Hz", "active carriers x spacing"],
        ["Passband centre", f"{P.centre_hz:.0f} Hz", "audio"],
        [
            "Full pilot symbols",
            f"every {P.pilot_symbol_period}th data symbol",
            "all carriers known",
        ],
        ["Preamble", f"{PREAMBLE_SYMBOLS} symbols", "two identical Schmidl-Cox symbols"],
        [
            "Mode/RV chips",
            f"{pre.n_chips}",
            f"{N_RV} x {air.n_modes} sequences, pairwise |correlation| <= "
            f"{air.chip_correlation_bound}",
        ],
        [
            "Acquisition threshold",
            f"{air.acquisition_threshold}",
            "normalised matched-filter peak",
        ],
    ]
    return _table(["Parameter", "Value", "Notes"], rows)


def layout_block(air: AirInterface = WIDE) -> str:
    rows = []
    for layout in air.layouts:
        rows.append(
            [
                layout.name.upper(),
                f"{layout.preamble_symbols} + {layout.data_symbols} = {layout.total_symbols}",
                f"{layout.duration_s * 1e3:.0f} ms",
                f"{layout.samples}",
                ", ".join(str(i) for i in layout.pilot_symbol_indices),
                f"{layout.qam_symbols}",
            ]
        )
    return _table(
        ["Layout", "Symbols", "Duration", "Samples (8 kHz)", "Full pilot symbols", "QAM slots"],
        rows,
    )


def mode_block(layout: FrameLayout = LONG, air: AirInterface = WIDE) -> str:
    """The air's ladder: every rung, the tone floor's kinds first (ADR-0013, ADR-0014)."""
    thresholds = AWGN_THRESHOLD_DB if air is WIDE else NARROW_AWGN_THRESHOLD_DB
    rows = []
    for r in air.ladder:
        if r.tone is not None:
            k = r.tone
            rows.append(
                [
                    str(r.index),
                    k.name,
                    f"TONE, {round(1 / k.data.symbol_s)} Bd data",
                    str(k.data.bits_per_symbol),
                    f"{k.rate:.2f}",
                    f"BG{k.base_graph}",
                    str(k.lifting_size),
                    str(k.info_bits),
                    str(k.coded_bits),
                    str(k.payload_bytes),
                    f"{k.net_bps:.0f}",
                    f"{thresholds[r.index]:+.1f}",
                ]
            )
            continue
        m = r.mode
        assert m is not None
        lay = air.data_layout(r.index)
        rows.append(
            [
                str(r.index),
                f"{m.name} (OFDM mode {m.index})",
                lay.name.upper(),
                str(m.modulation.bits_per_symbol),
                str(m.code_rate),
                f"BG{m.base_graph(lay)}",
                str(m.lifting_size(lay)),
                str(m.info_bits(lay)),
                str(m.coded_bits(lay)),
                str(m.payload_bytes(lay)),
                f"{m.net_bit_rate(lay):.0f}",
                f"{thresholds[r.index]:+.1f}",
            ]
        )
    return _table(
        [
            "Rung",
            "Name",
            "Frame",
            "bits/sym",
            "Rate",
            "Base graph",
            "Z",
            "K'",
            "E",
            "Payload B",
            "Net bps",
            "AWGN dB",
        ],
        rows,
    )


def tone_block() -> str:
    """The tone floor's numerology, frame kinds and detector constants (ADR-0013), and its
    fast kinds' data numerologies (ADR-0014)."""
    num = TONE_NUMEROLOGY
    fast = sorted({k.data for k in TONE_FAST}, key=lambda n: -n.symbol_samples)
    det = tone.ToneDetector()
    rows = [
        ["Tones", f"{num.tones}", f"{num.bits_per_symbol} Gray-labelled coded bits a symbol"],
        [
            "Symbol",
            f"{num.symbol_samples} samples ({num.symbol_s * 1e3:.0f} ms)",
            f"{1 / num.symbol_s:.0f} Bd",
        ],
        [
            "Tone spacing",
            f"{num.spacing_hz:.0f} Hz",
            f"tones at (k - {(num.tones - 1) / 2}) x spacing about the passband centre",
        ],
        ["Span", f"{num.span_hz:.0f} Hz", "lowest tone to highest, plus a spacing"],
        [
            "Tone change",
            f"{num.ramp_samples} samples",
            "raised-cosine frequency glide centred on the boundary; continuous phase",
        ],
        ["Frame edges", f"{num.edge_samples} samples", "raised-cosine amplitude fade in and out"],
        *(
            [
                f"Fast data, {1 / n.symbol_s:.0f} Bd",
                f"{n.symbol_samples} samples ({n.symbol_s * 1e3:.0f} ms), "
                f"{n.spacing_hz:.0f} Hz apart, span {n.span_hz:.0f} Hz",
                f"{num.symbol_samples // n.symbol_samples} data symbols a slot; tone change "
                f"{n.ramp_samples} samples, the shorter glide at a boundary with a sync symbol",
            ]
            for n in fast
        ),
        [
            "Level",
            f"+{tone.TONE_GAIN_DB:.1f} dB",
            "over an OFDM frame's average power at the same transmit level",
        ],
        [
            "Sync blocks",
            f"3 x {SYNC_SYMBOLS} symbols",
            "start, middle, end; 45 % of the data slots before the middle one; the same "
            "for every kind",
        ],
        [
            "Detector",
            f"hop {num.symbol_samples // det.HOP_DIV} samples, bin {det.bin_hz:.2f} Hz",
            f"offset search +/-{det.cfo_bins * det.bin_hz:.0f} Hz",
        ],
        [
            "Acquisition threshold",
            f"{det.threshold}",
            f"mean sync-tone ratio, each clipped at {det.CLIP:.0f}; "
            f"{det.MIN_HITS} of 24 sync tones strongest, {det.MIN_BLOCK_HITS} of them in a "
            "second block; a silent symbol is no evidence",
        ],
        [
            "Arrival threshold",
            f"{tone.ANNOUNCE_THRESHOLD}",
            f"first block's mean ratio; {det.MIN_FIRST_HITS} of 8 strongest",
        ],
    ]
    kinds = []
    for k in (TONE_CONTROL, *TONE_DATA, *TONE_FAST):
        data = (
            str(k.data_symbols)
            if k.speed == 1
            else f"{k.data_symbols} at {1 / k.data.symbol_s:.0f} Bd in {k.data_slots}"
        )
        kinds.append(
            [
                k.name,
                str(k.payload_bytes),
                f"{data} + 3 x {SYNC_SYMBOLS} = {k.symbols}",
                f"{k.duration_s:.2f} s",
                ", ".join(str(o) for o in k.block_offsets),
                f"{k.rate:.2f}",
                f"{k.net_bps:.1f}",
                ", ".join(str(p) for p in k.patterns),
                f"{TONE_CONTROL_THRESHOLD_DB:+.1f}"
                if k.control
                else f"{AWGN_THRESHOLD_DB[WIDE.tone_data.index(k)]:+.1f}",
            ]
        )
    patterns = [[str(i), " ".join(str(t) for t in p)] for i, p in enumerate(SYNC_PATTERNS)]
    return "\n\n".join(
        (
            _table(["Parameter", "Value", "Notes"], rows),
            _table(
                [
                    "Kind",
                    "Payload B",
                    "Data + sync = slots",
                    "Duration",
                    "Sync blocks at",
                    "Rate",
                    "Net bps",
                    "Patterns (by RV)",
                    "AWGN dB",
                ],
                kinds,
            ),
            _table(["Pattern", "Tones"], patterns),
        )
    )


def constants_block() -> str:
    rows = [
        [
            "Payload CRC",
            f"{PAYLOAD_CRC.name}, polynomial 0x{PAYLOAD_CRC.poly:06X}, {PAYLOAD_CRC.width} bits",
        ],
        ["Schmidl-Cox PN seed, DATA", str(SC_SEEDS[0])],
        ["Schmidl-Cox PN seed, CONTROL", str(SC_SEEDS[1])],
        ["Mode/RV chip seed", str(MODE_CHIP_SEED)],
        ["Redundancy versions", str(N_RV)],
        ["Peak reduction target, PSK modes", f"{CLIP_TARGET_DB:.1f} dB"],
        ["Peak reduction target, QAM modes", f"{CLIP_TARGET_DENSE_DB:.1f} dB"],
        ["Link DATA header", f"{DATA_HEADER} bytes (5 with an explicit length)"],
        ["Link CONTROL frame", f"{CONTROL_BYTES} bytes"],
        ["Callsign encoding", f"6 bits/character, 9 characters in {CALL_BYTES} bytes"],
        ["Selective-repeat window", f"{WINDOW} frames"],
    ]
    return _table(["Constant", "Value"], rows)


BLOCKS = {
    "waveform": waveform_block,
    "layouts": layout_block,
    "modes": mode_block,
    "constants": constants_block,
    "waveform500": lambda: waveform_block(NARROW),
    "layouts500": lambda: layout_block(NARROW),
    "modes500": lambda: mode_block(NARROW.long, NARROW),
    "tone": tone_block,
}


def render(text: str) -> str:
    for name, builder in BLOCKS.items():
        pattern = re.compile(rf"<!-- BEGIN:{name} -->.*?<!-- END:{name} -->", re.S)
        if not pattern.search(text):
            raise SystemExit(f"marker BEGIN:{name} missing from {SPEC}")
        block = f"<!-- BEGIN:{name} -->\n{builder()}\n<!-- END:{name} -->"
        text = pattern.sub(lambda _m, replacement=block: replacement, text)
    return text


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if the spec is out of date")
    args = ap.parse_args()

    if not SPEC.exists():
        raise SystemExit(f"{SPEC} does not exist")
    original = SPEC.read_text(encoding="utf-8")
    updated = render(original)
    if args.check:
        if original != updated:
            print(f"{SPEC.name} is out of date - run: python tools/make_spec.py")
            return 1
        print(f"{SPEC.name} is up to date")
        return 0
    if original != updated:
        SPEC.write_text(updated, encoding="utf-8", newline="\n")
        print(f"updated {SPEC}")
    else:
        print(f"{SPEC.name} already up to date")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
